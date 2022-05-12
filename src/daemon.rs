use crate::{onchain, p2p};
use futures::future::try_join_all;
use libp2p::identity::{secp256k1, Keypair};
use log::info;
use std::error::Error;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use url::Url;
use web3::types::Address;

pub struct DaemonOpts {
  pub eth_addr: Url,
  pub master_addr: Option<Address>,
  pub rpc_addr: SocketAddr,
  pub s3_addr: Option<SocketAddr>,
}

pub async fn listen_and_serve(opts: DaemonOpts) -> Result<(), Box<dyn std::error::Error>> {
  info!("initializing p2pim");

  let secp256k1_keypair = secp256k1::Keypair::generate();
  let keypair = Keypair::Secp256k1(secp256k1_keypair.clone());
  let p2p = p2p::create_p2p(keypair).await?;

  type ServeFuture = Pin<Box<dyn Future<Output = Result<(), Box<dyn Error>>>>>;

  let data = crate::data::new_service();
  let private_key_raw = secp256k1_keypair.secret().to_bytes();

  let onchain = crate::onchain::new_service(onchain::OnchainParams {
    eth_url: opts.eth_addr,
    private_key: private_key_raw,
    master_address: opts.master_addr,
  })
  .await?;

  let persistence = crate::persistence::new_service();

  let (reactor, reactor_fut) = crate::reactor::new_service(data, onchain.clone(), p2p.clone(), persistence.clone());

  let grpc: ServeFuture = Box::pin(crate::grpc::listen_and_serve(
    opts.rpc_addr,
    onchain.clone(),
    p2p.clone(),
    reactor.clone(),
    persistence.clone(),
  ));

  let s3 = opts
    .s3_addr
    .map::<ServeFuture, _>(|addr| Box::pin(crate::s3::listen_and_serve(addr)));
  let reactor_fut2: ServeFuture = Box::pin(futures::FutureExt::map(reactor_fut, Result::Ok));
  let futures: Vec<ServeFuture> = vec![Some(reactor_fut2), Some(grpc), s3].into_iter().flatten().collect();
  try_join_all(futures).await.map(|_| ())
}
