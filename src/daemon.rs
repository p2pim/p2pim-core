use std::future::Future;
use std::net::SocketAddr;

use log::{debug, info};
use p2pim_ethereum_contracts;
use p2pim_ethereum_contracts::{third::openzeppelin, P2pimAdjudicator};
use url::Url;
use web3::types::Address;

pub struct DaemonOpts {
  pub eth_addr: Url,
  pub rpc_addr: SocketAddr,
  pub master_addr: Option<Address>,
}

pub async fn listen_and_serve(opts: DaemonOpts) -> Result<(), Box<dyn std::error::Error>> {
  match opts.eth_addr.scheme() {
    "unix" => {
      listen_and_serve1(
        &opts.eth_addr,
        opts.rpc_addr,
        opts.master_addr,
        web3::transports::ipc::Ipc::new(opts.eth_addr.path()),
      )
      .await
    }
    "ws" | "wss" => {
      listen_and_serve1(
        &opts.eth_addr,
        opts.rpc_addr,
        opts.master_addr,
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
  transport_fut: impl Future<Output = Result<T, web3::Error>>,
) -> Result<(), Box<dyn std::error::Error>>
where
  F: std::future::Future<Output = web3::Result<serde_json::Value>> + Send + 'static,
  B: std::future::Future<Output = web3::Result<Vec<web3::Result<serde_json::Value>>>> + Send + 'static,
  T: web3::Transport<Out = F> + web3::BatchTransport<Batch = B> + Send + Sync + 'static,
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
  let account = accounts.get(0).map(Clone::clone).ok_or("no accounts configured")?;
  debug!("using account {:?}", account);

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

  crate::grpc::listen_and_serve(rpc_addr, account, deployments).await?;
  Ok(())
}
