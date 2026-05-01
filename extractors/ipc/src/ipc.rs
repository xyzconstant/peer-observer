#[allow(dead_code)]
mod generated {
    capnp::generated_code!(pub mod proxy_capnp, "capnp/mp/proxy_capnp.rs");
    capnp::generated_code!(pub mod common_capnp, "capnp/common_capnp.rs");
    capnp::generated_code!(pub mod echo_capnp, "capnp/echo_capnp.rs");
    capnp::generated_code!(pub mod mining_capnp, "capnp/mining_capnp.rs");
    capnp::generated_code!(pub mod handler_capnp, "capnp/handler_capnp.rs");
    capnp::generated_code!(pub mod chain_capnp, "capnp/chain_capnp.rs");
    capnp::generated_code!(pub mod rpc_capnp, "capnp/rpc_capnp.rs");
    capnp::generated_code!(pub mod init_capnp, "capnp/init_capnp.rs");
}
use generated::*;

use chain_capnp::chain::Client as ChainClient;
use chain_capnp::chain_notifications;
use handler_capnp::handler::Client as HandlerClient;
use init_capnp::init::Client as InitClient;
use mining_capnp::mining::Client as MiningClient;
use proxy_capnp::thread::Client as ThreadClient;
use proxy_capnp::thread_map::Client as ThreadMapClient;

use capnp_rpc::{Disconnector, RpcSystem, rpc_twoparty_capnp, twoparty};
use shared::{
    futures::AsyncReadExt,
    protobuf::ipc_extractor::BlockTip,
    tokio::{self, net::UnixStream, task::JoinHandle},
    tokio_util,
};

use crate::error::RuntimeError;

struct ChainNotificationsImpl;

impl chain_notifications::Server for ChainNotificationsImpl {
    async fn destroy(
        self: capnp::capability::Rc<Self>,
        _: chain_notifications::DestroyParams,
        _: chain_notifications::DestroyResults,
    ) -> Result<(), capnp::Error> {
        Ok(())
    }

    async fn transaction_added_to_mempool(
        self: capnp::capability::Rc<Self>,
        _: chain_notifications::TransactionAddedToMempoolParams,
        _: chain_notifications::TransactionAddedToMempoolResults,
    ) -> Result<(), capnp::Error> {
        Ok(())
    }

    async fn transaction_removed_from_mempool(
        self: capnp::capability::Rc<Self>,
        _: chain_notifications::TransactionRemovedFromMempoolParams,
        _: chain_notifications::TransactionRemovedFromMempoolResults,
    ) -> Result<(), capnp::Error> {
        Ok(())
    }

    async fn block_connected(
        self: capnp::capability::Rc<Self>,
        _: chain_notifications::BlockConnectedParams,
        _: chain_notifications::BlockConnectedResults,
    ) -> Result<(), capnp::Error> {
        Ok(())
    }

    async fn block_disconnected(
        self: capnp::capability::Rc<Self>,
        _: chain_notifications::BlockDisconnectedParams,
        _: chain_notifications::BlockDisconnectedResults,
    ) -> Result<(), capnp::Error> {
        Ok(())
    }

    async fn updated_block_tip(
        self: capnp::capability::Rc<Self>,
        _: chain_notifications::UpdatedBlockTipParams,
        _: chain_notifications::UpdatedBlockTipResults,
    ) -> Result<(), capnp::Error> {
        Ok(())
    }

    async fn chain_state_flushed(
        self: capnp::capability::Rc<Self>,
        _: chain_notifications::ChainStateFlushedParams,
        _: chain_notifications::ChainStateFlushedResults,
    ) -> Result<(), capnp::Error> {
        Ok(())
    }
}

pub struct IpcClient {
    pub reader: IpcReader,
    pub listener: IpcListener,
    pub rpc_task: JoinHandle<Result<(), capnp::Error>>,
    pub disconnector: Disconnector<rpc_twoparty_capnp::Side>,
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
        let disconnector = rpc_system.get_disconnector();
        let rpc_task = tokio::task::spawn_local(rpc_system);

        let thread_map: ThreadMapClient = init
            .construct_request()
            .send()
            .promise
            .await?
            .get()?
            .get_thread_map()?;
        let thread: ThreadClient = thread_map
            .make_thread_request()
            .send()
            .promise
            .await?
            .get()?
            .get_result()?;

        let mut req = init.make_mining_request();
        set_context(req.get().get_context()?, &thread);
        let mining: MiningClient = req.send().promise.await?.get()?.get_result()?;

        let mut req = init.make_chain_request();
        set_context(req.get().get_context()?, &thread);
        let chain: ChainClient = req.send().promise.await?.get()?.get_result()?;

        let mut req = chain.handle_notifications_request();
        set_context(req.get().get_context()?, &thread);
        req.get()
            .set_notifications(capnp_rpc::new_client(ChainNotificationsImpl));
        let handler: HandlerClient = req.send().promise.await?.get()?.get_result()?;

        Ok(Self {
            reader: IpcReader {
                mining,
                thread: thread.clone(),
            },
            listener: IpcListener { handler, thread },
            rpc_task,
            disconnector,
        })
    }
}

#[derive(Clone)]
pub struct IpcReader {
    pub mining: MiningClient,
    pub thread: ThreadClient,
}

impl IpcReader {
    pub async fn get_tip(&self) -> Result<Option<BlockTip>, RuntimeError> {
        let mut req = self.mining.get_tip_request();
        set_context(req.get().get_context()?, &self.thread);
        let response = req.send().promise.await?;

        let has_result = response.get()?.get_has_result();
        if !has_result {
            return Ok(None);
        }

        let tip = response.get()?.get_result()?;
        let height = tip.get_height();
        let hash = tip.get_hash()?.to_vec();

        Ok(Some(BlockTip { height, hash }))
    }
}

fn set_context(mut ctx: proxy_capnp::context::Builder<'_>, thread: &ThreadClient) {
    ctx.set_thread(thread.clone());
    ctx.set_callback_thread(thread.clone());
}

pub struct IpcListener {
    pub handler: HandlerClient,
    pub thread: ThreadClient,
}

impl IpcListener {
    pub async fn shutdown(&self) -> Result<(), RuntimeError> {
        let mut req = self.handler.disconnect_request();
        set_context(req.get().get_context()?, &self.thread);
        req.send().promise.await?;
        Ok(())
    }
}
