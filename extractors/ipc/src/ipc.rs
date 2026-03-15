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

use crate::error::RuntimeError;

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

        let response = init.construct_request().send().promise.await?;
        let thread_map = response.get()?.get_thread_map()?;

        let response = thread_map.make_thread_request().send().promise.await?;
        let thread = response.get()?.get_result()?;

        let mut req = init.make_mining_request();
        {
            let mut ctx = req.get().get_context()?;
            ctx.set_thread(thread.clone());
            ctx.set_callback_thread(thread.clone());
        }
        let response = req.send().promise.await?;
        let mining = response.get()?.get_result()?;

        Ok(Self {
            rpc_task,
            thread,
            mining,
        })
    }

    pub async fn get_tip(&self) -> Result<BlockTip, RuntimeError> {
        let mut req = self.mining.get_tip_request();
        {
            let mut ctx = req.get().get_context()?;
            ctx.set_thread(self.thread.clone());
            ctx.set_callback_thread(self.thread.clone());
        }

        let response = req.send().promise.await?;
        let tip = response.get()?.get_result()?;

        let height = tip.get_height();
        let mut hash = tip.get_hash()?.to_vec();
        hash.reverse();

        Ok(BlockTip { height, hash })
    }
}
