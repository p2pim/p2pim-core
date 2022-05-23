use std::convert::TryInto;
use std::error::Error;
use std::fmt::Debug;
use std::net::SocketAddr;
use std::time::{Duration, SystemTime};

use crate::proto::api::balance_entry::{StorageBalance, TokenMetadata, WalletBalance};
use crate::proto::api::list_storage_rented_response::StorageRentedData;
use crate::proto::api::p2pim_server::{P2pim, P2pimServer};
use crate::proto::api::swarm_server::{Swarm, SwarmServer};
use crate::proto::api::{
  ApproveRequest, ApproveResponse, BalanceEntry, ChallengeRequest, ChallengeResponse, DepositRequest, DepositResponse,
  GetBalanceRequest, GetBalanceResponse, GetConnectedPeersRequest, GetConnectedPeersResponse, GetInfoRequest,
  GetInfoResponse, ListStorageRentedRequest, ListStorageRentedResponse, RetrieveRequest, RetrieveResponse, StoreRequest,
  StoreResponse, WithdrawRequest, WithdrawResponse,
};
use crate::proto::libp2p::PeerId;
use crate::types::{Balance, ChallengeKey, LeaseTerms};
use crate::{onchain, p2p, persistence, reactor};
use futures::StreamExt;
use log::info;
use tonic::transport::Server;
use tonic::{Request, Response, Status};
use web3::types::Address;

pub async fn listen_and_serve<TOnchain, TP2p, TPersistence, TReactor>(
  rpc_addr: SocketAddr,
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
    onchain,
    persistence,
    reactor,
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
  onchain: TOnchain,
  persistence: TPersistence,
  reactor: TReactor,
}

#[tonic::async_trait]
impl<TOnchain, TPersistence, TReactor> P2pim for P2pimImpl<TOnchain, TPersistence, TReactor>
where
  TOnchain: onchain::Service,
  TPersistence: persistence::Service,
  TReactor: reactor::Service,
{
  async fn get_info(&self, _: Request<GetInfoRequest>) -> Result<Response<GetInfoResponse>, Status> {
    let balance = futures::stream::iter(self.onchain.deployed_tokens().await.iter())
      .then(|(token_address, _)| async move {
        self
          .onchain
          .balance(token_address)
          .await
          .map(|b| convert_balance(*token_address, b))
      })
      .collect::<Vec<Result<BalanceEntry, _>>>()
      .await
      .into_iter()
      .collect::<Result<Vec<BalanceEntry>, _>>()
      .map_err(|e| Status::internal(format!("[TODO(formatting)] {}", e)))?;

    Ok(Response::new(GetInfoResponse {
      address_wallet: Some(From::from(&self.onchain.account_wallet())),
      address_storage: Some(From::from(&self.onchain.account_storage())),
      balance,
    }))
  }

  async fn get_balance(&self, request: Request<GetBalanceRequest>) -> Result<Response<GetBalanceResponse>, Status> {
    let token_addr: web3::types::Address = request
      .get_ref()
      .token_address
      .as_ref()
      .ok_or(Status::invalid_argument("token_address empty"))?
      .into();

    let balance = self
      .onchain
      .balance(&token_addr)
      .await
      .map(|b| convert_balance(token_addr, b))
      .map_err(|e| Status::internal(format!("[TODO(formatting)] {}", e)))?;

    Ok(Response::new(GetBalanceResponse { balance: Some(balance) }))
  }

  async fn approve(&self, request: Request<ApproveRequest>) -> Result<Response<ApproveResponse>, Status> {
    let token_addr = request
      .get_ref()
      .token_address
      .as_ref()
      .ok_or(Status::invalid_argument("token_address empty"))?
      .into();

    let result = self
      .onchain
      .approve(&token_addr)
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

    let result = self
      .onchain
      .deposit(&token_addr, amount)
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

  async fn retrieve(&self, request: Request<RetrieveRequest>) -> Result<Response<RetrieveResponse>, Status> {
    let req = request.get_ref();
    let peer_id = req
      .peer_id
      .as_ref()
      .ok_or(Status::invalid_argument("peer empty"))?
      .try_into()
      .map_err(|e| Status::invalid_argument(format!("invalid peer id: {}", e)))?;
    let nonce = req.nonce;
    let data = self
      .reactor
      .retrieve(peer_id, nonce)
      .await
      .map_err(|e| Status::unknown(format!("error retrieving the data: {}", e)))?;
    Ok(Response::new(RetrieveResponse { data }))
  }

  async fn challenge(&self, request: Request<ChallengeRequest>) -> Result<Response<ChallengeResponse>, Status> {
    let req = request.get_ref();
    let peer_id = req
      .peer_id
      .as_ref()
      .ok_or(Status::invalid_argument("peer empty"))?
      .try_into()
      .map_err(|e| Status::invalid_argument(format!("invalid peer id: {}", e)))?;
    let nonce = req.nonce;
    let block_number = req.block_number;
    self
      .reactor
      .challenge(peer_id, ChallengeKey { nonce, block_number })
      .await
      .map_err(|e| Status::unknown(format!("error challenging a lease: {}", e)))?;
    Ok(Response::new(ChallengeResponse {}))
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
          lease_started: l.chain_confirmation.map(|c| c.timestamp.into()),
        })
        .collect(),
    }))
  }
}

fn convert_balance(token_address: Address, balance: Balance) -> BalanceEntry {
  BalanceEntry {
    token_address: Some(token_address.into()),
    token_metadata: balance.token_metadata.map(|m| TokenMetadata {
      symbol: m.symbol,
      name: m.name,
      decimals: m.decimals as u32,
    }),
    storage_balance: Some(StorageBalance {
      available: Some(balance.storage_balance.available.into()),
      locked_rents: Some(balance.storage_balance.locked_rents.into()),
      locked_lets: Some(balance.storage_balance.locked_lets.into()),
    }),
    wallet_balance: Some(WalletBalance {
      available: Some(balance.wallet_balance.available.into()),
      allowance: Some(balance.wallet_balance.allowance.into()),
    }),
  }
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
