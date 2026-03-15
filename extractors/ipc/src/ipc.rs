#[allow(dead_code)]
mod generated {
    capnp::generated_code!(pub mod proxy_capnp, "capnp/mp/proxy_capnp.rs");
    capnp::generated_code!(pub mod common_capnp, "capnp/common_capnp.rs");
    capnp::generated_code!(pub mod mining_capnp, "capnp/mining_capnp.rs");
    capnp::generated_code!(pub mod echo_capnp, "capnp/echo_capnp.rs");
    capnp::generated_code!(pub mod init_capnp, "capnp/init_capnp.rs");
}
use generated::*;

use init_capnp::init::Client as InitClient;
use mining_capnp::mining::Client as MiningClient;
use proxy_capnp::thread::Client as ThreadClient;

use capnp_rpc::{RpcSystem, rpc_twoparty_capnp, twoparty};
use shared::{
    futures::AsyncReadExt,
    protobuf::ipc_extractor::BlockTip,
    tokio::{self, net::UnixStream, task::JoinHandle},
};

use crate::error::{IpcCallKind, RuntimeError};

pub struct IpcClient {
    pub mining: MiningClient,
    pub thread: ThreadClient,
    pub rpc_task: JoinHandle<Result<(), capnp::Error>>,
}

impl IpcClient {
    pub async fn init(stream: UnixStream) -> Result<Self, crate::error::RuntimeError> {
        let (reader, writer) = tokio_util::compat::TokioAsyncReadCompatExt::compat(stream).split();
        let network = Box::new(twoparty::VatNetwork::new(
            reader,
            writer,
            rpc_twoparty_capnp::Side::Client,
            Default::default(),
        ));

        let mut rpc_system = RpcSystem::new(network, None);
        let init: InitClient = rpc_system.bootstrap(rpc_twoparty_capnp::Side::Server);
        let rpc_task = tokio::task::spawn_local(rpc_system);

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

        let mut make_mining_request = init.make_mining_request();
        {
            let mut context = make_mining_request
                .get()
                .get_context()
                .map_err(|e| RuntimeError::ipc_call(IpcCallKind::InitMakeMining, e))?;
            context.set_thread(thread.clone());
            context.set_callback_thread(thread.clone());
        }
        let response = make_mining_request
            .send()
            .promise
            .await
            .map_err(|e| RuntimeError::ipc_call(IpcCallKind::InitMakeMining, e))?;
        let mining = response
            .get()
            .map_err(|e| RuntimeError::ipc_call(IpcCallKind::InitMakeMining, e))?
            .get_result()
            .map_err(|e| RuntimeError::ipc_call(IpcCallKind::InitMakeMining, e))?;

        Ok(Self {
            rpc_task,
            thread,
            mining,
        })
    }

    pub async fn get_tip(&self) -> Result<BlockTip, RuntimeError> {
        let mut get_tip_request = self.mining.get_tip_request();
        {
            let mut context = get_tip_request
                .get()
                .get_context()
                .map_err(|e| RuntimeError::ipc_call(IpcCallKind::MiningGetTip, e))?;
            context.set_thread(self.thread.clone());
            context.set_callback_thread(self.thread.clone());
        }

        let get_tip_response = get_tip_request
            .send()
            .promise
            .await
            .map_err(|e| RuntimeError::ipc_call(IpcCallKind::MiningGetTip, e))?;

        let height: i32;
        let mut hash: Vec<u8>;
        {
            let tip = get_tip_response
                .get()
                .map_err(|e| RuntimeError::ipc_call(IpcCallKind::MiningGetTip, e))?
                .get_result()
                .map_err(|e| RuntimeError::ipc_call(IpcCallKind::MiningGetTip, e))?;

            height = tip.get_height();
            hash = tip
                .get_hash()
                .map_err(|e| RuntimeError::ipc_call(IpcCallKind::MiningGetTip, e))?
                .to_vec();
            hash.reverse();
        }

        Ok(BlockTip { height, hash })
    }
}
