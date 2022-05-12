use crate::{onchain, p2p};
use futures::future::try_join_all;
use libp2p::identity::{secp256k1, Keypair};
use log::{debug, info};

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

  let secp256k1_keypair = secp256k1::Keypair::generate();
  let keypair = Keypair::Secp256k1(secp256k1_keypair.clone());
  let p2p = p2p::create_p2p(keypair).await?;

  type ServeFuture = Pin<Box<dyn Future<Output = Result<(), Box<dyn Error>>>>>;

  let data = crate::data::new_service();
  let private_key_raw = secp256k1_keypair.secret().to_bytes();
  let onchain = crate::onchain::new_service(
    onchain::OnchainParams {
      private_key: private_key_raw,
      master_address: master_addr,
    },
    web3,
  )
  .await?;

  let persistence = crate::persistence::new_service();

  let (reactor, reactor_fut) = crate::reactor::new_service(data, onchain.clone(), p2p.clone(), persistence.clone());

  let grpc: ServeFuture = Box::pin(crate::grpc::listen_and_serve(
    rpc_addr,
    onchain.clone(),
    p2p.clone(),
    reactor.clone(),
    persistence.clone(),
  ));

  let s3 = s3_addr.map::<ServeFuture, _>(|addr| Box::pin(crate::s3::listen_and_serve(addr)));
  let reactor_fut2: ServeFuture = Box::pin(futures::FutureExt::map(reactor_fut, Result::Ok));
  let futures: Vec<ServeFuture> = vec![Some(reactor_fut2), Some(grpc), s3].into_iter().flatten().collect();
  try_join_all(futures).await.map(|_| ())
}
