use crate::utils::ethereum::IntoAddress;
use crate::{onchain, p2p};
use futures::future::try_join_all;
use libp2p::identity::{secp256k1, Keypair};
use log::{debug, info};
use p2pim_ethereum_contracts;
use p2pim_ethereum_contracts::{third::openzeppelin, P2pimAdjudicator};
use std::convert::identity;
use std::error::Error;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use url::Url;
use web3::types::Address;
use web3::DuplexTransport;

pub struct DaemonOpts {
  pub eth_addr: Url,
  pub master_addr: Option<Address>,
  pub rpc_addr: SocketAddr,
  pub s3_addr: Option<SocketAddr>,
}

pub async fn listen_and_serve(opts: DaemonOpts) -> Result<(), Box<dyn std::error::Error>> {
  match opts.eth_addr.scheme() {
    "unix" => {
      listen_and_serve1(
        &opts.eth_addr,
        opts.rpc_addr,
        opts.master_addr,
        opts.s3_addr,
        web3::transports::ipc::Ipc::new(opts.eth_addr.path()),
      )
      .await
    }
    "ws" | "wss" => {
      listen_and_serve1(
        &opts.eth_addr,
        opts.rpc_addr,
        opts.master_addr,
        opts.s3_addr,
        web3::transports::ws::WebSocket::new(opts.eth_addr.as_str()),
      )
      .await
    }
    unsupported => Err(format!("unsupported schema: {}", unsupported).into()),
  }
}

async fn listen_and_serve1<F, B, T>(
  eth_addr: &Url,
  rpc_addr: SocketAddr,
  master_addr: Option<Address>,
  s3_addr: Option<SocketAddr>,
  transport_fut: impl Future<Output = Result<T, web3::Error>>,
) -> Result<(), Box<dyn std::error::Error>>
where
  F: std::future::Future<Output = web3::Result<serde_json::Value>> + Send + 'static,
  B: std::future::Future<Output = web3::Result<Vec<web3::Result<serde_json::Value>>>> + Send + 'static,
  T: web3::Transport<Out = F> + web3::BatchTransport<Batch = B> + DuplexTransport + Send + Sync + 'static,
  <T as DuplexTransport>::NotificationStream: std::marker::Send + Unpin,
{
  info!("initializing p2pim");

  debug!("creating transport using {}", eth_addr);
  let transport = transport_fut.await?;

  debug!("creating web3");
  let web3 = web3::Web3::new(transport);

  let network_id = web3.net().version().await?;
  info!("connected to eth network with id {}", network_id);

  debug!("initializing master record contract");
  let instance = if let Some(addr) = master_addr {
    Ok(p2pim_ethereum_contracts::P2pimMasterRecord::at(&web3, addr))
  } else {
    p2pim_ethereum_contracts::P2pimMasterRecord::deployed(&web3).await
  }?;
  debug!("using master record contract on address {}", instance.address());

  debug!("reading accounts");
  let accounts = web3.eth().accounts().await?;
  let account_wallet = accounts.get(0).map(Clone::clone).ok_or("no accounts configured")?;
  debug!("using account for wallet {:?}", account_wallet);

  // TODO react to new deployments
  debug!("reading master record deployments");
  let deployments = instance
    .methods()
    .deployments()
    .call()
    .await?
    .into_iter()
    .map(|(token, adjudicator_addr)| {
      (
        openzeppelin::IERC20Metadata::at(&web3, token),
        P2pimAdjudicator::at(&web3, adjudicator_addr),
      )
    })
    .collect();
  debug!("found deployments {:?}", deployments);

  let secp256k1_keypair = secp256k1::Keypair::generate();
  let keypair = Keypair::Secp256k1(secp256k1_keypair.clone());
  let account_storage = secp256k1_keypair.public().into_address();
  let p2p = p2p::create_p2p(keypair).await?;

  type ServeFuture = Pin<Box<dyn Future<Output = Result<(), Box<dyn Error>>>>>;

  let data = crate::data::new_service();
  let private_key_raw = secp256k1_keypair.secret().to_bytes();
  let onchain = crate::onchain::new_service(
    onchain::OnchainParams {
      private_key: private_key_raw,
      master_address: master_addr.clone(),
    },
    web3,
  )
  .await?;
  let (reactor, reactor_fut) = crate::reactor::new_service(data, onchain, p2p.clone());

  let grpc: ServeFuture = Box::pin(crate::grpc::listen_and_serve(
    rpc_addr,
    account_wallet,
    account_storage,
    deployments,
    p2p.clone(),
    reactor.clone(),
  ));

  let s3 = s3_addr.map::<ServeFuture, _>(|addr| Box::pin(crate::s3::listen_and_serve(addr)));
  let reactor_fut2: ServeFuture = Box::pin(futures::FutureExt::map(reactor_fut, Result::Ok));
  let futures: Vec<ServeFuture> = vec![Some(reactor_fut2), Some(grpc), s3]
    .into_iter()
    .filter_map(identity)
    .collect();
  try_join_all(futures).await.map(|_| ())
}
