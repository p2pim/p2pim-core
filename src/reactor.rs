use crate::p2p::p2pim::LeaseProposal;
use crate::p2p::Event;
use crate::types::{ChainConfirmation, ChallengeKey, ChallengeProof, Lease, LeaseTerms};
use crate::utils::ethereum::IntoAddress;
use crate::{cryptography, data, lessor, onchain, p2p, persistence};
use anyhow::anyhow;
use ethcontract::transaction::TransactionResult;
use ethcontract::{EventMetadata, EventStatus};
use futures::future::join_all;
use futures::{select, FutureExt, StreamExt};
use libp2p::PeerId;
use log::{error, info, trace, warn};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, SystemTime};
use tonic::async_trait;
use web3::types::{BlockId, H256};

#[async_trait]
pub trait Service: Clone + Send + Sync + 'static {
  async fn lease(&self, peer_id: PeerId, terms: LeaseTerms, data: Vec<u8>) -> Result<H256, Box<dyn Error>>;
  async fn challenge(&self, peer_id: PeerId, challenge_key: ChallengeKey) -> Result<(), Box<dyn Error>>;
  async fn retrieve(&self, peer_id: PeerId, nonce: u64) -> anyhow::Result<Vec<u8>>;
}

#[derive(Clone)]
struct Implementation<TData, TLessor, TOnchain, TP2p, TPersistence>
where
  TData: data::Service,
  TLessor: lessor::Service,
  TOnchain: onchain::Service,
  TP2p: p2p::Service,
  TPersistence: persistence::Service,
{
  data: TData,
  lessor: TLessor,
  onchain: TOnchain,
  p2p: TP2p,
  persistence: TPersistence,
}

pub fn new_service<TData, TLessor, TOnchain, TP2p, TPersistence>(
  data: TData,
  lessor: TLessor,
  onchain: TOnchain,
  p2p: TP2p,
  persistence: TPersistence,
) -> (impl Service, impl Future<Output = ()>)
where
  TData: data::Service,
  TLessor: lessor::Service,
  TOnchain: onchain::Service,
  TP2p: p2p::Service,
  TPersistence: persistence::Service,
{
  let implementation = Implementation {
    data,
    lessor,
    onchain,
    p2p,
    persistence,
  };

  type ReactorFuture = Pin<Box<dyn Future<Output = ()>>>;

  let p2p_fut: ReactorFuture = Box::pin(implementation.clone().process_p2p_events());
  let onchain_fut: ReactorFuture = Box::pin(implementation.clone().process_onchain_events());
  let futures = vec![p2p_fut, onchain_fut];
  (implementation, join_all(futures).map(|_| ()))
}

enum ProcessProposalError {
  Rejected(lessor::RejectedReason),
  OnchainError(onchain::Error),
  DataError(anyhow::Error),
}

impl From<onchain::Error> for ProcessProposalError {
  fn from(value: onchain::Error) -> Self {
    ProcessProposalError::OnchainError(value)
  }
}

impl From<anyhow::Error> for ProcessProposalError {
  fn from(value: anyhow::Error) -> Self {
    ProcessProposalError::DataError(value)
  }
}

impl Display for ProcessProposalError {
  fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
    match self {
      ProcessProposalError::Rejected(reason) => write!(f, "proposal rejected: {}", reason),
      ProcessProposalError::OnchainError(err) => {
        write!(f, "onchain error: {}", err)
      }
      ProcessProposalError::DataError(err) => {
        write!(f, "data error: {}", err)
      }
    }
  }
}

impl<TData, TLessor, TOnchain, TP2p, TPersistence> Implementation<TData, TLessor, TOnchain, TP2p, TPersistence>
where
  TData: data::Service,
  TLessor: lessor::Service,
  TOnchain: onchain::Service,
  TP2p: p2p::Service,
  TPersistence: persistence::Service,
{
  async fn process_p2p_events(mut self) {
    while let Some(ev) = self.p2p.next().await {
      match ev {
        Event::ReceivedLeaseProposal { peer_id, proposal } => {
          let self_clone = self.clone();
          tokio::task::spawn(async move {
            let nonce = proposal.nonce;
            match self_clone.process_proposal_received(peer_id, proposal).await {
              Ok(TransactionResult::Hash(hash)) => info!("lease sealed transaction_hash={}", hash),
              Ok(TransactionResult::Receipt(receipt)) => info!("lease sealed transaction_hash={}", receipt.transaction_hash),
              Err(ProcessProposalError::Rejected(reason)) => {
                self_clone
                  .p2p
                  .send_proposal_rejection(peer_id, nonce, reason.to_string())
                  .await;
              }
              Err(err) => {
                error!("unexpected error while processing lease proposal: {}", err);
              }
            }
          });
        }
        Event::ReceivedChallengeRequest { peer_id, challenge_key } => {
          let self_clone = self.clone();
          tokio::task::spawn(async move {
            let result = self_clone.send_proof(peer_id, challenge_key).await;
            if let Err(e) = result {
              error!("TODO (Handling): error while trying to send proof: {:?}", e);
            }
          });
        }
        Event::ReceivedRetrieveRequest { peer_id, nonce } => {
          let self_clone = self.clone();
          tokio::task::spawn(async move {
            let result = self_clone.send_retrieve_delivery(peer_id, nonce).await;
            if let Err(e) = result {
              error!("TODO (Handling): error while trying to send data: {:?}", e);
            }
          });
        }
      }
    }
  }

  async fn process_onchain_events(self) {
    let mut events_stream = self.onchain.listen_adjudicator_events().await;
    while let Some(ev) = events_stream.next().await {
      match ev {
        Err(e) => error!("TODO: reactor: error receiving onchain events: {}", e),
        Ok(ethcontract::Event { data, meta: Some(meta) }) => {
          let result = self.process_onchain_event(data, meta).await;
          if let Err(e) = result {
            error!("reactor: error processing onchain event: {}", e)
          }
        }
        Ok(ethcontract::Event { meta: None, .. }) => {
          unreachable!("we are not looking for not confirmed events")
        }
      }
    }
  }

  async fn process_proposal_received(
    &self,
    peer_id: PeerId,
    proposal: LeaseProposal,
  ) -> Result<TransactionResult, ProcessProposalError>
  where
    TData: data::Service,
    TOnchain: onchain::Service,
    TP2p: p2p::Service,
  {
    if let Err(e) = self
      .lessor
      .proposal(&peer_id, &proposal.lease_terms, proposal.data.len())
      .await
    {
      return Err(ProcessProposalError::Rejected(e));
    }

    let lessee_address = self
      .p2p
      .find_public_key(&peer_id)
      .as_ref()
      .map(IntoAddress::into_address)
      .expect("peer id should be identified already");
    // TODO check if the nonce is duplicated
    let data_parameters = self.data.store(peer_id, proposal.nonce, proposal.data.as_slice()).await?;

    let result = self
      .onchain
      .seal_lease(
        lessee_address,
        proposal.nonce,
        proposal.lease_terms,
        data_parameters,
        proposal.signature,
      )
      .await?;
    info!("lease sealed peer_id={} transaction_result={:?}", peer_id, result);
    Ok(result)
  }

  async fn send_proof(&self, peer_id: PeerId, challenge_key: ChallengeKey) -> Result<(), Box<dyn Error>> {
    let (block_data, proof) = self
      .data
      .proof(peer_id, challenge_key.nonce, challenge_key.block_number as usize)
      .await?;
    self
      .p2p
      .send_challenge_proof(peer_id, challenge_key, ChallengeProof { block_data, proof })
      .await;
    Ok(())
  }

  async fn send_retrieve_delivery(&self, peer_id: PeerId, nonce: u64) -> anyhow::Result<()> {
    let data = self.data.retrieve(peer_id, nonce).await?;
    self.p2p.send_retrieve_delivery(peer_id, nonce, data).await;

    Ok(())
  }

  async fn process_onchain_event(
    &self,
    event: EventStatus<p2pim_ethereum_contracts::adjudicator::event_data::LeaseSealed>,
    meta: EventMetadata,
  ) -> Result<(), Box<dyn Error>> {
    let own_address = self.onchain.account_storage();
    let block = self
      .onchain
      .block(BlockId::Hash(meta.block_hash))
      .await?
      .ok_or("block not found")?;
    //let block = self.onchain
    match event {
      EventStatus::Removed(ev) if ev.lessee == own_address => self
        .persistence
        .rent_update_chain(ev.lessor, ev.nonce, None)
        .await
        .map_err(|_| "lease not found")?,
      EventStatus::Removed(ev) if ev.lessor == own_address => warn!("TODO: handle lets events"),
      EventStatus::Added(ev) if ev.lessee == own_address => self
        .persistence
        .rent_update_chain(
          ev.lessor,
          ev.nonce,
          Some(ChainConfirmation {
            transaction_hash: meta.transaction_hash,
            timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(block.timestamp.as_u64()),
          }),
        )
        .await
        .unwrap_or_else(|err| error!("reactor: error processing a onchain event: {}: {:?}", err, ev)),
      EventStatus::Added(ev) if ev.lessor == own_address => warn!("TODO: handle lets events"),
      _ => error!("received event does not belong to us: {:?}", event),
    };
    Ok(())
  }
}

#[async_trait]
impl<TData, TLessor, TOnchain, TP2p, TPersistence> Service for Implementation<TData, TLessor, TOnchain, TP2p, TPersistence>
where
  TData: data::Service,
  TLessor: lessor::Service,
  TOnchain: onchain::Service,
  TP2p: p2p::Service,
  TPersistence: persistence::Service,
{
  async fn lease(&self, peer_id: PeerId, terms: LeaseTerms, data: Vec<u8>) -> Result<H256, Box<dyn Error>> {
    let nonce = rand::random(); // TODO Is this ok?
    let data_parameters = self.data.parameters(data.as_slice()).await;
    let lessor_address = self
      .p2p
      .find_public_key(&peer_id)
      .as_ref()
      .map(IntoAddress::into_address)
      .ok_or("peer id not found")?;
    let signature = self
      .onchain
      .sign_proposal(&lessor_address, nonce, &terms, &data_parameters)
      .await;

    let expiration = terms.proposal_expiration;
    let token_address = terms.token_address;

    self
      .persistence
      .rent_store(Lease {
        peer_id,
        peer_address: lessor_address,
        nonce,
        terms: terms.clone(),
        data_parameters: data_parameters.clone(),
        chain_confirmation: None,
      })
      .await;

    let mut p2p_future = self.p2p.send_proposal(peer_id, nonce, terms, signature, data).fuse();

    let mut seal_lease_future = self
      .onchain
      .wait_for_seal_lease(&token_address, lessor_address, nonce, expiration)
      .fuse();

    select! {
      reason = p2p_future => Err(format!("lease rejected with reason: {}, note that the lease can still be processed on chain", reason).into()),
      e = seal_lease_future =>  {
        match e {
          Ok(Some(ev)) => {
            if ev.is_removed() {
              todo!()
            } else {
              Ok(ev.meta.expect("we not look for transactions not confirmed").transaction_hash)
            }
          }
          Ok(None) => Err("lease timed out".into()),
          Err(e) => Err(e.into()),
        }
      }
    }
  }

  async fn challenge(&self, peer_id: PeerId, challenge_key: ChallengeKey) -> Result<(), Box<dyn Error>> {
    let ChallengeKey { nonce, block_number } = challenge_key;
    let lease = self.persistence.rent_get(peer_id, nonce).await.ok_or("lease not found")?;
    if lease.data_parameters.size < (block_number as usize) * cryptography::BLOCK_SIZE_BYTES {
      return Err("block number is out of bounds".into());
    }

    // TODO timeout
    let challenge_proof = self.p2p.challenge(peer_id, challenge_key.clone()).await?;
    trace!("proof received peer={}", peer_id);

    let valid = self
      .data
      .verify(
        lease.data_parameters,
        block_number,
        challenge_proof.block_data.as_slice(),
        challenge_proof.proof,
      )
      .await;
    if valid {
      Ok(())
    } else {
      Err("proof not valid".into())
    }
  }

  async fn retrieve(&self, peer_id: PeerId, nonce: u64) -> anyhow::Result<Vec<u8>> {
    let lease = self
      .persistence
      .rent_get(peer_id, nonce)
      .await
      .ok_or_else(|| anyhow!("lease not found"))?;
    let data = self.p2p.retrieve(peer_id, nonce).await?;
    let parameters = self.data.parameters(data.as_slice()).await;
    if parameters.size != lease.data_parameters.size {
      Err(anyhow!(
        "unexpected data size, expected={}, received={}",
        lease.data_parameters.size,
        parameters.size
      ))
    } else if parameters.merkle_root != lease.data_parameters.merkle_root {
      Err(anyhow!("received data does not match with the merkle root"))
    } else {
      Ok(data)
    }
  }
}
