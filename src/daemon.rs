use std::fmt::Debug;
use std::future::Future;
use std::net::SocketAddr;

use crate::proto::api::p2pim_server::{P2pim, P2pimServer};
use crate::proto::api::{ApproveRequest, ApproveResponse, BalanceEntry, GetInfoRequest, GetInfoResponse, TokenInfo};
use futures::StreamExt;
use log::{debug, info, warn};
use p2pim_ethereum_contracts;
use p2pim_ethereum_contracts::{third::openzeppelin, P2pimAdjudicator};
use tonic::transport::Server;
use tonic::{Request, Response, Status};
use url::Url;
use web3::types::{Address, U256};

#[derive(Clone, Debug)]
struct P2pimImpl {
  account: web3::types::Address,
  deployments: Vec<(openzeppelin::IERC20Metadata, P2pimAdjudicator)>,
}

#[tonic::async_trait]
impl P2pim for P2pimImpl {
  async fn get_info(&self, _: Request<GetInfoRequest>) -> Result<Response<GetInfoResponse>, Status> {
    let balance = futures::stream::iter(self.deployments.iter())
      .then(|(token, adjudicator)| async move { read_balances(self.account, token, adjudicator).await })
      .collect::<Vec<Result<BalanceEntry, _>>>()
      .await
      .into_iter()
      .collect::<Result<Vec<BalanceEntry>, _>>()
      .map_err(|e| Status::internal(format!("[TODO(formatting)] {}", e.to_string())))?;

    Ok(Response::new(GetInfoResponse {
      address: Some(From::from(&self.account)),
      balance,
    }))
  }

  async fn approve(&self, request: Request<ApproveRequest>) -> Result<Response<ApproveResponse>, Status> {
    let token_addr = request
      .get_ref()
      .token_address
      .as_ref()
      .ok_or(Status::invalid_argument("token_address empty"))?
      .into();

    match self.deployments.iter().find(|(t, _)| t.address() == token_addr) {
      None => Err(Status::invalid_argument("adjudicator not found for the token")),
      Some((token, adjudicator)) => {
        let result = token
          .approve(adjudicator.address(), U256::max_value())
          .confirmations(0)
          .send()
          .await
          .map_err(|e| Status::internal(format!("error sending approval transaction: {}", e)))?;
        Ok(Response::new(ApproveResponse {
          transaction_hash: Some(From::from(result.hash())),
        }))
      }
    }
  }
}

async fn read_balances(
  account: web3::types::Address,
  token: &openzeppelin::IERC20Metadata,
  adjudicator: &P2pimAdjudicator,
) -> Result<BalanceEntry, ethcontract::errors::MethodError> {
  fn ok_or_warn<R, E: std::fmt::Display>(result: Result<R, E>, method: &str, address: web3::types::Address) -> Option<R> {
    if let Err(e) = result.as_ref() {
      warn!("error calling `{}` method error={} address={}", method, e, address)
    }
    result.ok()
  }

  let supplied = adjudicator.balance(account).call().await?.0;
  let name = ok_or_warn(token.name().call().await, "name", token.address());
  let symbol = ok_or_warn(token.methods().symbol().call().await, "symbol", token.address());
  let decimals = ok_or_warn(token.methods().decimals().call().await, "symbol", token.address());

  let available = token.balance_of(account).call().await?;
  let allowance = token.allowance(account, adjudicator.address()).call().await?;

  Ok(BalanceEntry {
    token: Some(TokenInfo {
      token_address: Some(From::from(&token.address())),
      name: name.unwrap_or(Default::default()),
      decimals: From::from(decimals.unwrap_or(Default::default())),
      symbol: symbol.unwrap_or(Default::default()),
    }),
    available: Some(From::from(&available)),
    allowed: Some(From::from(&allowance)),
    supplied: Some(From::from(&supplied)),
  })
}

pub async fn listen_and_serve(
  eth_addr: Url,
  rpc_addr: SocketAddr,
  master_addr: Option<Address>,
) -> Result<(), Box<dyn std::error::Error>> {
  let mut builder = Server::builder();
  let router = match eth_addr.scheme() {
    "unix" => {
      let p2pim_impl = initialize_p2pim(&eth_addr, web3::transports::ipc::Ipc::new(eth_addr.path()), master_addr).await?;
      Ok(builder.add_service(P2pimServer::new(p2pim_impl)))
    }
    "ws" | "wss" => {
      let p2pim_impl = initialize_p2pim(
        &eth_addr,
        web3::transports::ws::WebSocket::new(eth_addr.as_str()),
        master_addr,
      )
      .await?;
      Ok(builder.add_service(P2pimServer::new(p2pim_impl)))
    }
    unsupported => Err(format!("unsupported schema: {}", unsupported)),
  }?;

  info!("starting gRPC server on {}", rpc_addr);
  router.serve(rpc_addr).await?;
  Ok(())
}

async fn initialize_p2pim<F, B, T>(
  eth_addr: &Url,
  transport_fut: impl Future<Output = Result<T, web3::Error>>,
  master_addr: Option<Address>,
) -> Result<P2pimImpl, Box<dyn std::error::Error>>
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

  Ok(P2pimImpl { account, deployments })
}
