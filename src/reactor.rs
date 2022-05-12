use crate::p2p::behaviour::Event;
use crate::p2p::p2pim::LeaseProposal;
use crate::types::{ChainConfirmation, Lease, LeaseTerms};
use crate::utils::ethereum::IntoAddress;
use crate::{data, onchain, p2p, persistence};
use ethcontract::transaction::TransactionResult;
use ethcontract::{EventMetadata, EventStatus};
use futures::future::join_all;
use futures::{FutureExt, StreamExt};
use libp2p::PeerId;
use log::{error, info, trace, warn};
use std::error::Error;
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, SystemTime};
use tonic::async_trait;
use web3::types::{BlockId, H256};

#[async_trait]
pub trait Service: Clone + Send + Sync + 'static {
  async fn lease(&self, peer_id: PeerId, terms: LeaseTerms, data: Vec<u8>) -> Result<H256, Box<dyn Error>>;
}

#[derive(Clone)]
struct Implementation<TData, TOnchain, TP2p, TPersistence>
where
  TData: data::Service,
  TOnchain: onchain::Service,
  TP2p: p2p::Service,
  TPersistence: persistence::Service,
{
  data: TData,
  onchain: TOnchain,
  p2p: TP2p,
  persistence: TPersistence,
}

pub fn new_service<TData, TOnchain, TP2p, TPersistence>(
  data: TData,
  onchain: TOnchain,
  p2p: TP2p,
  persistence: TPersistence,
) -> (impl Service, impl Future<Output = ()>)
where
  TData: data::Service,
  TOnchain: onchain::Service,
  TP2p: p2p::Service,
  TPersistence: persistence::Service,
{
  let implementation = Implementation {
    data,
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

impl<TData, TOnchain, TP2p, TPersistence> Implementation<TData, TOnchain, TP2p, TPersistence>
where
  TData: data::Service,
  TOnchain: onchain::Service,
  TP2p: p2p::Service,
  TPersistence: persistence::Service,
{
  async fn process_p2p_events(mut self) {
    while let Some(ev) = self.p2p.next().await {
      trace!("p2p: {:?}", ev);
      match ev {
        Event::ReceivedLeaseProposal { peer_id, proposal } => {
          let self_clone = self.clone();
          tokio::task::spawn(async move {
            let result = self_clone.seal_lease(peer_id, proposal).await;
            if let Err(e) = result {
              warn!("error while trying to seal deal: {}", e);
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
            error!("reactor: error processing event: {}", e)
          }
        }
        Ok(ethcontract::Event { meta: None, .. }) => {
          unreachable!("we are not looking for not confirmed events")
        }
      }
    }
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

  async fn seal_lease(&self, peer_id: PeerId, proposal: LeaseProposal) -> Result<TransactionResult, Box<dyn Error>>
  where
    TData: data::Service,
    TOnchain: onchain::Service,
    TP2p: p2p::Service,
  {
    let lessee_address = self
      .p2p
      .find_public_key(&peer_id)
      .as_ref()
      .map(IntoAddress::into_address)
      .ok_or("peer id not found")?;
    let data_parameters = self.data.parameters(proposal.data.as_slice()).await;

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
}

#[async_trait]
impl<TData, TOnchain, TP2p, TPersistence> Service for Implementation<TData, TOnchain, TP2p, TPersistence>
where
  TData: data::Service,
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

    self.p2p.send_proposal(peer_id, nonce, terms, signature, data).await;

    let seal_lease_event = self
      .onchain
      .wait_for_seal_lease(&token_address, lessor_address, nonce, expiration)
      .await?;

    match seal_lease_event {
      Some(ev) => {
        if ev.is_removed() {
          todo!()
        } else {
          Ok(ev.meta.expect("we not look for transactions not confirmed").transaction_hash)
        }
      }
      None => Err("lease timed out".into()),
    }
  }
}
