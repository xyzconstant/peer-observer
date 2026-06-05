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
use proxy_capnp::thread_map::Client as ThreadMapClient;

use capnp_rpc::{Disconnector, RpcSystem, rpc_twoparty_capnp, twoparty};
use shared::{
    bitcoin::{self, consensus::Decodable, hashes::Hash},
    futures::AsyncReadExt,
    protobuf::{
        bitcoin_primitives,
        ipc_extractor::{
            BlockConnected, BlockDisconnected, BlockInfo, BlockTip, ChainStateFlushed,
            ChainstateRole, TransactionAddedToMempool, TransactionRemovedFromMempool,
        },
    },
    tokio::{self, net::UnixStream, task::JoinHandle},
    tokio_util,
};

use std::future::Future;
use std::pin::Pin;

use crate::error::RuntimeError;

pub struct IpcClient {
    pub reader: IpcReader,
    pub rpc_task: JoinHandle<Result<(), capnp::Error>>,
    pub disconnector: Disconnector<rpc_twoparty_capnp::Side>,
    init: InitClient,
}

impl IpcClient {
    pub async fn connect(stream: UnixStream) -> Result<Self, crate::error::RuntimeError> {
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

        let mut pool_req = thread_map.make_pool_request();
        pool_req.get().set_count(2);
        pool_req.send().promise.await?;

        let req = init.make_mining_request();
        let mining: MiningClient = req.send().promise.await?.get()?.get_result()?;

        let reader = IpcReader {
            mining,
        };

        Ok(Self {
            reader,
            rpc_task,
            disconnector,
            init,
        })
    }

    pub async fn subscribe_chain_notifications(
        &self,
        callbacks: ChainCallbacks,
    ) -> Result<IpcListener, crate::error::RuntimeError> {
        let req = self.init.make_chain_request();
        let chain: ChainClient = req.send().promise.await?.get()?.get_result()?;

        let mut req = chain.handle_notifications_request();
        req.get()
            .set_notifications(capnp_rpc::new_client(ChainNotificationsImpl { callbacks }));
        let handler: HandlerClient = req.send().promise.await?.get()?.get_result()?;

        Ok(IpcListener {
            handler,
        })
    }
}

#[derive(Clone)]
pub struct IpcReader {
    pub mining: MiningClient,
}

impl IpcReader {
    pub async fn get_tip(&self) -> Result<Option<BlockTip>, RuntimeError> {
        let req = self.mining.get_tip_request();
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

pub struct IpcListener {
    pub handler: HandlerClient,
}

impl IpcListener {
    pub async fn shutdown(&self) -> Result<(), RuntimeError> {
        let req = self.handler.disconnect_request();
        req.send().promise.await?;
        Ok(())
    }
}

pub type EventFut = Pin<Box<dyn Future<Output = ()>>>;

pub struct ChainCallbacks {
    pub on_block_connected: Box<dyn Fn(BlockConnected) -> EventFut>,
    pub on_block_disconnected: Box<dyn Fn(BlockDisconnected) -> EventFut>,
    pub on_tx_added: Box<dyn Fn(TransactionAddedToMempool) -> EventFut>,
    pub on_tx_removed: Box<dyn Fn(TransactionRemovedFromMempool) -> EventFut>,
    pub on_chain_state_flushed: Box<dyn Fn(ChainStateFlushed) -> EventFut>,
    pub on_updated_block_tip: Box<dyn Fn() -> EventFut>,
}

struct ChainNotificationsImpl {
    callbacks: ChainCallbacks,
}

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
        params: chain_notifications::TransactionAddedToMempoolParams,
        _: chain_notifications::TransactionAddedToMempoolResults,
    ) -> Result<(), capnp::Error> {
        let tx = parse_transaction(params.get()?.get_tx()?)?;
        (self.callbacks.on_tx_added)(TransactionAddedToMempool { tx }).await;
        Ok(())
    }

    async fn transaction_removed_from_mempool(
        self: capnp::capability::Rc<Self>,
        params: chain_notifications::TransactionRemovedFromMempoolParams,
        _: chain_notifications::TransactionRemovedFromMempoolResults,
    ) -> Result<(), capnp::Error> {
        let r = params.get()?;
        let tx = parse_transaction(r.get_tx()?)?;
        let reason = r.get_reason();
        (self.callbacks.on_tx_removed)(TransactionRemovedFromMempool { tx, reason }).await;
        Ok(())
    }

    async fn block_connected(
        self: capnp::capability::Rc<Self>,
        params: chain_notifications::BlockConnectedParams,
        _: chain_notifications::BlockConnectedResults,
    ) -> Result<(), capnp::Error> {
        let r = params.get()?;
        let role = parse_chainstate_role(r.get_role()?);
        let block = parse_block_info(r.get_block()?)?;
        (self.callbacks.on_block_connected)(BlockConnected { role, block }).await;
        Ok(())
    }

    async fn block_disconnected(
        self: capnp::capability::Rc<Self>,
        params: chain_notifications::BlockDisconnectedParams,
        _: chain_notifications::BlockDisconnectedResults,
    ) -> Result<(), capnp::Error> {
        let block = parse_block_info(params.get()?.get_block()?)?;
        (self.callbacks.on_block_disconnected)(BlockDisconnected { block }).await;
        Ok(())
    }

    async fn updated_block_tip(
        self: capnp::capability::Rc<Self>,
        _: chain_notifications::UpdatedBlockTipParams,
        _: chain_notifications::UpdatedBlockTipResults,
    ) -> Result<(), capnp::Error> {
        (self.callbacks.on_updated_block_tip)().await;
        Ok(())
    }

    async fn chain_state_flushed(
        self: capnp::capability::Rc<Self>,
        params: chain_notifications::ChainStateFlushedParams,
        _: chain_notifications::ChainStateFlushedResults,
    ) -> Result<(), capnp::Error> {
        let r = params.get()?;
        let role = parse_chainstate_role(r.get_role()?);
        let locator = r.get_locator()?.to_vec();
        (self.callbacks.on_chain_state_flushed)(ChainStateFlushed { role, locator }).await;
        Ok(())
    }
}

fn parse_chainstate_role(r: chain_capnp::chainstate_role::Reader<'_>) -> ChainstateRole {
    ChainstateRole {
        validated: r.get_validated(),
        historical: r.get_historical(),
    }
}

fn parse_block_info(r: chain_capnp::block_info::Reader<'_>) -> Result<BlockInfo, capnp::Error> {
    Ok(BlockInfo {
        height: r.get_height(),
        hash: r.get_hash()?.to_vec(),
        prev_hash: r.get_prev_hash()?.to_vec(),
        chain_time_max: Some(r.get_chain_time_max()),
    })
}

fn parse_transaction(raw: &[u8]) -> Result<bitcoin_primitives::Transaction, capnp::Error> {
    let parsed = bitcoin::Transaction::consensus_decode(&mut &raw[..])
        .map_err(|e| capnp::Error::failed(format!("tx decode failed: {e}")))?;
    let txid = parsed.compute_txid().to_byte_array().to_vec();
    let wtxid = parsed.compute_wtxid().to_byte_array().to_vec();
    Ok(bitcoin_primitives::Transaction {
        txid,
        wtxid,
        raw: Some(raw.to_vec()),
    })
}
