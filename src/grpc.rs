use std::error::Error;
use std::fmt::Debug;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use crate::proto::api::p2pim_server::{P2pim, P2pimServer};
use crate::proto::api::swarm_server::{Swarm, SwarmServer};
use crate::proto::api::{
  ApproveRequest, ApproveResponse, BalanceEntry, DepositRequest, DepositResponse, GetBalanceRequest, GetBalanceResponse,
  GetConnectedPeersRequest, GetConnectedPeersResponse, GetInfoRequest, GetInfoResponse, PeerId, TokenInfo,
};
use futures::StreamExt;
use log::{info, warn};
use p2pim_ethereum_contracts;
use p2pim_ethereum_contracts::{third::openzeppelin, P2pimAdjudicator};
use tonic::transport::Server;
use tonic::{Request, Response, Status};
use web3::types::U256;

pub async fn listen_and_serve(
  rpc_addr: SocketAddr,
  account: web3::types::Address,
  deployments: Vec<(openzeppelin::IERC20Metadata, P2pimAdjudicator)>,
  swarm: Arc<Mutex<libp2p::swarm::Swarm<impl libp2p::swarm::NetworkBehaviour + Send>>>,
) -> Result<(), Box<dyn Error>> {
  info!("starting gRPC server on {}", rpc_addr);
  let p2pim_impl = P2pimImpl { account, deployments };
  let swarm_impl = SwarmImpl { swarm };
  Server::builder()
    .add_service(P2pimServer::new(p2pim_impl))
    .add_service(SwarmServer::new(swarm_impl))
    .serve(rpc_addr)
    .await
    .map_err(|e| e.into())
}

#[derive(Clone, Debug)]
struct P2pimImpl {
  account: web3::types::Address,
  deployments: Vec<(openzeppelin::IERC20Metadata, P2pimAdjudicator)>,
}

impl P2pimImpl {
  fn deployment(&self, token_addr: web3::types::Address) -> Option<&(openzeppelin::IERC20Metadata, P2pimAdjudicator)> {
    self.deployments.iter().find(|(t, _)| t.address() == token_addr)
  }
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

  async fn get_balance(&self, request: Request<GetBalanceRequest>) -> Result<Response<GetBalanceResponse>, Status> {
    let token_addr = request
      .get_ref()
      .token_address
      .as_ref()
      .ok_or(Status::invalid_argument("token_address empty"))?
      .into();
    let (token, adjudicator) = self
      .deployment(token_addr)
      .ok_or(Status::not_found("adjudicator not found for the token"))?;
    let balance = read_balances(token_addr, token, adjudicator)
      .await
      .map_err(|e| Status::internal(format!("[TODO(formatting)] {}", e.to_string())))?;
    Ok(Response::new(GetBalanceResponse { balance: Some(balance) }))
  }

  async fn approve(&self, request: Request<ApproveRequest>) -> Result<Response<ApproveResponse>, Status> {
    let token_addr = request
      .get_ref()
      .token_address
      .as_ref()
      .ok_or(Status::invalid_argument("token_address empty"))?
      .into();

    let (token, adjudicator) = self
      .deployment(token_addr)
      .ok_or(Status::invalid_argument("adjudicator not found for the token"))?;

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

  async fn deposit(&self, request: Request<DepositRequest>) -> Result<Response<DepositResponse>, Status> {
    let token_addr = request
      .get_ref()
      .token_address
      .as_ref()
      .ok_or(Status::invalid_argument("token_address empty"))?
      .into();

    let amount = request
      .get_ref()
      .amount
      .as_ref()
      .ok_or(Status::invalid_argument("amount empty"))?
      .into();

    let (_, adjudicator) = self
      .deployment(token_addr)
      .ok_or(Status::invalid_argument("adjudicator not found for the token"))?;
    let result = adjudicator
      .deposit(amount, self.account)
      .confirmations(0)
      .send()
      .await
      .map_err(|e| Status::internal(format!("error sending deposit transaction: {}", e)))?;
    Ok(Response::new(DepositResponse {
      transaction_hash: Some(From::from(result.hash())),
    }))
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

struct SwarmImpl<T: libp2p::swarm::NetworkBehaviour + Send> {
  swarm: Arc<Mutex<libp2p::swarm::Swarm<T>>>,
}

#[tonic::async_trait]
impl<T: libp2p::swarm::NetworkBehaviour + Send> Swarm for SwarmImpl<T> {
  async fn get_connected_peers(
    &self,
    _: Request<GetConnectedPeersRequest>,
  ) -> Result<Response<GetConnectedPeersResponse>, Status> {
    let swarm = self.swarm.lock().unwrap();
    let peer_list = swarm.connected_peers().map(|p| PeerId { data: p.to_bytes() }).collect();
    Ok(Response::new(GetConnectedPeersResponse { peer_list }))
  }
}
