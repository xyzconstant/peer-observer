capnp::generated_code!(pub mod proxy_capnp, "capnp/mp/proxy_capnp.rs");
capnp::generated_code!(pub mod common_capnp, "capnp/common_capnp.rs");
capnp::generated_code!(pub mod mining_capnp, "capnp/mining_capnp.rs");
capnp::generated_code!(pub mod echo_capnp, "capnp/echo_capnp.rs");
capnp::generated_code!(pub mod handler_capnp, "capnp/handler_capnp.rs");
capnp::generated_code!(pub mod chain_capnp, "capnp/chain_capnp.rs");
capnp::generated_code!(pub mod init_capnp, "capnp/init_capnp.rs");

use crate::init_capnp::init;
use crate::proxy_capnp::thread;
use crate::chain_capnp::chain;
use capnp_rpc::{RpcSystem, rpc_twoparty_capnp, twoparty};
use shared::clap::{self, Parser};
use shared::futures::AsyncReadExt;
use shared::nats_subjects::Subject;
use shared::nats_util::{self, NatsArgs};
use shared::prost::Message;
use shared::protobuf::event::{Event, event::PeerObserverEvent};
use shared::protobuf::ipc_extractor;
use shared::tokio::net::UnixStream;
use shared::tokio::sync::watch;
use shared::tokio::time::{self, Duration};
use shared::{async_nats, log};
use std::io;

mod error;
use error::{IpcCallKind, RuntimeError};

/// The peer-observer ipc-extractor periodically queries data from the
/// Bitcoin Core IPC interface and publishes the results as events into
/// a NATS pub-sub queue.
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Arguments for the connection to the NATS server.
    #[command(flatten)]
    pub nats: nats_util::NatsArgs,

    /// The log level the extractor should run with. Valid log levels are "trace",
    /// "debug", "info", "warn", "error". See https://docs.rs/log/latest/log/enum.Level.html.
    #[arg(short, long, default_value_t = log::Level::Debug)]
    pub log_level: log::Level,

    /// A UNIX socket path to read IPC data from.
    #[arg(short, long)]
    pub ipc_socket_path: String,

    /// Interval (in seconds) in which to query from the Bitcoin Core IPC interface.
    #[arg(long, default_value_t = 10)]
    pub query_interval: u64,
}

impl Args {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        nats: NatsArgs,
        log_level: log::Level,
        ipc_socket_path: String,
        query_interval: u64,
    ) -> Args {
        Self {
            nats,
            log_level,
            ipc_socket_path,
            query_interval,
        }
    }
}

struct IpcSession {
    chain: chain::Client,
    thread: thread::Client,
    rpc_task: shared::tokio::task::JoinHandle<Result<(), capnp::Error>>,
}

pub async fn run(args: Args, mut shutdown_rx: watch::Receiver<bool>) -> Result<(), RuntimeError> {
    let nats_client = nats_util::prepare_connection(&args.nats)?
        .connect(&args.nats.address)
        .await?;
    log::info!("Connected to NATS server at {}", &args.nats.address);

    let ipc_session = init_ipc_session(&args.ipc_socket_path).await?;

    let duration_sec = Duration::from_secs(args.query_interval);
    let mut interval = time::interval(duration_sec);
    log::info!(
        "Querying the Bitcoin Core IPC interface every {:?}.",
        duration_sec
    );

    loop {
        shared::tokio::select! {
            _ = interval.tick() => {
                if let Err(e) = get_height(&ipc_session, &nats_client).await {
                        log::error!("Could not fetch and publish 'getHeight': {}", e)
                    }
            }
            res = shutdown_rx.changed() => {
                match res {
                    Ok(_) => {
                        if *shutdown_rx.borrow() {
                            log::info!("ipc_extractor received shutdown signal.");
                            ipc_session.rpc_task.abort();
                            break;
                        }
                    }
                    Err(_) => {
                        // all senders dropped -> treat as shutdown
                        log::warn!("The shutdown notification sender was dropped. Shutting down.");
                        ipc_session.rpc_task.abort();
                        break;
                    }
                }
            }
        }
    }
    Ok(())
}

async fn init_ipc_session(ipc_socket_path: &str) -> Result<IpcSession, RuntimeError> {
    let stream = UnixStream::connect(ipc_socket_path).await.map_err(|e| {
        io::Error::new(
            e.kind(),
            format!(
                "could not connect to IPC socket at --ipc-socket-path '{}': {}",
                ipc_socket_path, e
            ),
        )
    })?;
    log::info!("Connected to IPC socket path at {}", ipc_socket_path);

    let (reader, writer) = tokio_util::compat::TokioAsyncReadCompatExt::compat(stream).split();
    let network = Box::new(twoparty::VatNetwork::new(
        reader,
        writer,
        rpc_twoparty_capnp::Side::Client,
        Default::default(),
    ));

    let mut rpc_system = RpcSystem::new(network, None);
    let init: init::Client = rpc_system.bootstrap(rpc_twoparty_capnp::Side::Server);
    let rpc_task = shared::tokio::task::spawn_local(async move { rpc_system.await });

    let response = init
        .construct_request()
        .send()
        .promise
        .await
        .map_err(|e| RuntimeError::ipc_call(IpcCallKind::InitConstruct, e))?;
    let thread_map = response
        .get()
        .map_err(|e| RuntimeError::ipc_call(IpcCallKind::InitConstruct, e))?
        .get_thread_map()
        .map_err(|e| RuntimeError::ipc_call(IpcCallKind::InitConstruct, e))?;

    let response = thread_map
        .make_thread_request()
        .send()
        .promise
        .await
        .map_err(|e| RuntimeError::ipc_call(IpcCallKind::ThreadMapMakeThread, e))?;
    let thread = response
        .get()
        .map_err(|e| RuntimeError::ipc_call(IpcCallKind::ThreadMapMakeThread, e))?
        .get_result()
        .map_err(|e| RuntimeError::ipc_call(IpcCallKind::ThreadMapMakeThread, e))?;

    let mut make_chain_request = init.make_chain_request();
    {
        let mut context = make_chain_request
            .get()
            .get_context()
            .map_err(|e| RuntimeError::ipc_call(IpcCallKind::InitMakeChain, e))?;
        context.set_thread(thread.clone());
        context.set_callback_thread(thread.clone());
    }
    let response = make_chain_request
        .send()
        .promise
        .await
        .map_err(|e| RuntimeError::ipc_call(IpcCallKind::InitMakeChain, e))?;
    let chain = response
        .get()
        .map_err(|e| RuntimeError::ipc_call(IpcCallKind::InitMakeChain, e))?
        .get_result()
        .map_err(|e| RuntimeError::ipc_call(IpcCallKind::InitMakeChain, e))?;

    Ok(IpcSession {
        rpc_task,
        thread,
        chain,
    })
}

async fn get_height(
    ipc_client: &IpcSession,
    nats_client: &async_nats::Client,
) -> Result<(), RuntimeError> {
    let mut get_height_request = ipc_client.chain.get_height_request();
    {
        let mut context = get_height_request
            .get()
            .get_context()
            .map_err(|e| RuntimeError::ipc_call(IpcCallKind::ChainGetHeight, e))?;
        context.set_thread(ipc_client.thread.clone());
        context.set_callback_thread(ipc_client.thread.clone());
    }

    let current_height = get_height_request
        .send()
        .promise
        .await
        .map_err(|e| RuntimeError::ipc_call(IpcCallKind::ChainGetHeight, e))?
        .get()
        .map_err(|e| RuntimeError::ipc_call(IpcCallKind::ChainGetHeight, e))?
        .get_result();

    let proto = Event::new(PeerObserverEvent::IpcExtractor(ipc_extractor::Ipc {
        ipc_event: Some(ipc_extractor::ipc::IpcEvent::CurrentHeight(current_height)),
    }))?;

    nats_client
        .publish(Subject::Ipc.to_string(), proto.encode_to_vec().into())
        .await?;
    Ok(())
}
