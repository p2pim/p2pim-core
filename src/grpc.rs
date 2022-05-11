use std::convert::TryInto;
use std::error::Error;
use std::fmt::Debug;
use std::net::SocketAddr;
use std::time::{Duration, SystemTime};

use crate::proto::api::list_storage_rented_response::StorageRentedData;
use crate::proto::api::p2pim_server::{P2pim, P2pimServer};
use crate::proto::api::swarm_server::{Swarm, SwarmServer};
use crate::proto::api::{
  ApproveRequest, ApproveResponse, BalanceEntry, DepositRequest, DepositResponse, GetBalanceRequest, GetBalanceResponse,
  GetConnectedPeersRequest, GetConnectedPeersResponse, GetInfoRequest, GetInfoResponse, ListStorageRentedRequest,
  ListStorageRentedResponse, StoreRequest, StoreResponse, TokenInfo, WithdrawRequest, WithdrawResponse,
};
use crate::proto::libp2p::PeerId;
use crate::types::LeaseTerms;
use crate::{onchain, p2p, persistence, reactor};
use futures::StreamExt;
use log::{info, warn};
use p2pim_ethereum_contracts;
use p2pim_ethereum_contracts::{third::openzeppelin, P2pimAdjudicator};
use tonic::transport::Server;
use tonic::{Request, Response, Status};
use web3::types::U256;

pub async fn listen_and_serve<TOnchain, TP2p, TPersistence, TReactor>(
  rpc_addr: SocketAddr,
  account_wallet: web3::types::Address,
  account_storage: web3::types::Address,
  deployments: Vec<(openzeppelin::IERC20Metadata, P2pimAdjudicator)>,
  onchain: TOnchain,
  p2p: TP2p,
  reactor: TReactor,
  persistence: TPersistence,
) -> Result<(), Box<dyn Error>>
where
  TOnchain: onchain::Service,
  TReactor: reactor::Service,
  TP2p: p2p::Service,
  TPersistence: persistence::Service,
{
  info!("starting gRPC server on {}", rpc_addr);
  let p2pim_impl = P2pimImpl {
    account_wallet,
    account_storage,
    deployments,
    onchain,
    reactor,
    persistence,
  };
  let swarm_impl = SwarmImpl { p2p };
  Server::builder()
    .add_service(P2pimServer::new(p2pim_impl))
    .add_service(SwarmServer::new(swarm_impl))
    .serve(rpc_addr)
    .await
    .map_err(|e| e.into())
}

#[derive(Clone, Debug)]
struct P2pimImpl<TOnchain, TPersistence, TReactor>
where
  TOnchain: onchain::Service,
  TPersistence: persistence::Service,
  TReactor: reactor::Service,
{
  account_wallet: web3::types::Address,
  account_storage: web3::types::Address,
  deployments: Vec<(openzeppelin::IERC20Metadata, P2pimAdjudicator)>,
  onchain: TOnchain,
  reactor: TReactor,
  persistence: TPersistence,
}

impl<TOnchain, TPersistence, TReactor> P2pimImpl<TOnchain, TPersistence, TReactor>
where
  TOnchain: onchain::Service,
  TPersistence: persistence::Service,
  TReactor: reactor::Service,
{
  fn deployment(&self, token_addr: web3::types::Address) -> Option<&(openzeppelin::IERC20Metadata, P2pimAdjudicator)> {
    self.deployments.iter().find(|(t, _)| t.address() == token_addr)
  }
}

#[tonic::async_trait]
impl<TOnchain, TPersistence, TReactor> P2pim for P2pimImpl<TOnchain, TPersistence, TReactor>
where
  TOnchain: onchain::Service,
  TPersistence: persistence::Service,
  TReactor: reactor::Service,
{
  async fn get_info(&self, _: Request<GetInfoRequest>) -> Result<Response<GetInfoResponse>, Status> {
    let balance = futures::stream::iter(self.deployments.iter())
      .then(|(token, adjudicator)| async move {
        read_balances(self.account_wallet, self.account_storage, token, adjudicator).await
      })
      .collect::<Vec<Result<BalanceEntry, _>>>()
      .await
      .into_iter()
      .collect::<Result<Vec<BalanceEntry>, _>>()
      .map_err(|e| Status::internal(format!("[TODO(formatting)] {}", e.to_string())))?;

    Ok(Response::new(GetInfoResponse {
      address_wallet: Some(From::from(&self.account_wallet)),
      address_storage: Some(From::from(&self.account_storage)),
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
    let balance = read_balances(self.account_wallet, self.account_storage, token, adjudicator)
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
    let dep_req = request.get_ref();
    let token_addr = dep_req
      .token_address
      .as_ref()
      .ok_or(Status::invalid_argument("token_address empty"))?
      .into();

    let amount = dep_req
      .amount
      .as_ref()
      .ok_or(Status::invalid_argument("amount empty"))?
      .into();

    let (_, adjudicator) = self
      .deployment(token_addr)
      .ok_or(Status::invalid_argument("adjudicator not found for the token"))?;
    let result = adjudicator
      .deposit(amount, self.account_storage)
      .confirmations(0)
      .send()
      .await
      .map_err(|e| Status::internal(format!("error sending deposit transaction: {}", e)))?;
    Ok(Response::new(DepositResponse {
      transaction_hash: Some(From::from(result.hash())),
    }))
  }

  async fn withdraw(&self, request: Request<WithdrawRequest>) -> Result<Response<WithdrawResponse>, Status> {
    let dep_req = request.get_ref();
    let token_addr = dep_req
      .token_address
      .as_ref()
      .ok_or(Status::invalid_argument("token_address empty"))?
      .into();

    let amount = dep_req
      .amount
      .as_ref()
      .ok_or(Status::invalid_argument("amount empty"))?
      .into();

    let result = self
      .onchain
      .withdraw(&token_addr, amount)
      .await
      .map_err(|e| Status::internal(format!("error sending withdraw transaction: {}", e)))?;
    Ok(Response::new(WithdrawResponse {
      transaction_hash: Some(From::from(result.hash())),
    }))
  }

  async fn store(&self, request: Request<StoreRequest>) -> Result<Response<StoreResponse>, Status> {
    let req = request.into_inner();
    let peer_id = req
      .peer_id
      .as_ref()
      .ok_or(Status::invalid_argument("peer_id empty"))?
      .try_into()
      .map_err(|e| Status::invalid_argument(format!("invalid peer id: {}", e)))?;

    let lease_term = LeaseTerms {
      lease_duration: req
        .lease_duration
        .clone()
        .ok_or(Status::invalid_argument("lease duration empty"))?
        .try_into()
        .map_err(|_| Status::invalid_argument("duration should be positive value"))?,
      token_address: req
        .token_address
        .as_ref()
        .ok_or(Status::invalid_argument("token address empty"))?
        .into(),
      proposal_expiration: SystemTime::now() + Duration::from_secs(120), // TODO fixed 2 minutes, this needs to be a parameter
      price: req.price.as_ref().ok_or(Status::invalid_argument("price empty"))?.into(),
      penalty: req.penalty.as_ref().ok_or(Status::invalid_argument("penalty empty"))?.into(),
    };

    let result = self
      .reactor
      .lease(peer_id, lease_term, req.data)
      .await
      .map_err(|e| Status::unknown(format!("Error trying to store: {}", e)))?;
    Ok(Response::new(StoreResponse {
      transaction_hash: Some(result.into()),
    }))
  }

  async fn list_storage_rented(
    &self,
    _: Request<ListStorageRentedRequest>,
  ) -> Result<Response<ListStorageRentedResponse>, Status> {
    let list = self.persistence.rent_list().await;
    Ok(Response::new(ListStorageRentedResponse {
      storage_rented_data: list
        .into_iter()
        .map(|l| StorageRentedData {
          nonce: l.nonce,
          peer_id: Some(l.peer_id.into()),
          token_address: Some(l.terms.token_address.into()),
          lease_duration: Some(l.terms.lease_duration.into()),
          price: Some(l.terms.price.into()),
          penalty: Some(l.terms.penalty.into()),
          proposal_expiration: Some(l.terms.proposal_expiration.into()),
          transaction_hash: l.chain_confirmation.clone().map(|c| c.transaction_hash.into()),
          lease_started: l.chain_confirmation.clone().map(|c| c.timestamp.into()),
        })
        .collect(),
    }))
  }
}

async fn read_balances(
  account_wallet: web3::types::Address,
  account_storage: web3::types::Address,
  token: &openzeppelin::IERC20Metadata,
  adjudicator: &P2pimAdjudicator,
) -> Result<BalanceEntry, ethcontract::errors::MethodError> {
  fn ok_or_warn<R, E: std::fmt::Display>(result: Result<R, E>, method: &str, address: web3::types::Address) -> Option<R> {
    if let Err(e) = result.as_ref() {
      warn!("error calling `{}` method error={} address={}", method, e, address)
    }
    result.ok()
  }

  let (available_p2pim, locked_rents, locked_lets) = adjudicator.balance(account_storage).call().await?;
  let name = ok_or_warn(token.name().call().await, "name", token.address());
  let symbol = ok_or_warn(token.methods().symbol().call().await, "symbol", token.address());
  let decimals = ok_or_warn(token.methods().decimals().call().await, "symbol", token.address());

  let available_account = token.balance_of(account_wallet).call().await?;
  let allowance_account = token.allowance(account_wallet, adjudicator.address()).call().await?;

  Ok(BalanceEntry {
    token: Some(TokenInfo {
      token_address: Some(From::from(&token.address())),
      name: name.unwrap_or(Default::default()),
      decimals: From::from(decimals.unwrap_or(Default::default())),
      symbol: symbol.unwrap_or(Default::default()),
    }),
    available_account: Some(From::from(&available_account)),
    allowed_account: Some(From::from(&allowance_account)),
    available_p2pim: Some(From::from(&available_p2pim)),
    locked_rents: Some(From::from(&locked_rents)),
    locked_lets: Some(From::from(&locked_lets)),
  })
}

struct SwarmImpl<TP2p>
where
  TP2p: p2p::Service,
{
  p2p: TP2p,
}

#[tonic::async_trait]
impl<TP2p> Swarm for SwarmImpl<TP2p>
where
  TP2p: p2p::Service,
{
  async fn get_connected_peers(
    &self,
    _: Request<GetConnectedPeersRequest>,
  ) -> Result<Response<GetConnectedPeersResponse>, Status> {
    let peer_list = self
      .p2p
      .known_peers()
      .into_iter()
      .map(|p| PeerId { data: p.to_bytes() })
      .collect();
    Ok(Response::new(GetConnectedPeersResponse { peer_list }))
  }
}
