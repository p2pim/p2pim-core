use crate::p2p::behaviour::Event;
use crate::p2p::p2pim::LeaseProposal;
use crate::types::{LeaseTerms, Signature};
use crate::utils::ethereum::IntoAddress;
use crate::{data, onchain, p2p};
use ethcontract::transaction::TransactionResult;
use futures::StreamExt;
use libp2p::PeerId;
use log::{trace, warn};
use std::error::Error;
use std::future::Future;
use std::ops::Deref;
use std::pin::Pin;
use tonic::async_trait;
use web3::types::H256;

#[async_trait]
pub trait Service: Clone + Send + Sync + 'static {
  async fn lease(&self, peer_id: PeerId, terms: LeaseTerms, data: Vec<u8>) -> Result<H256, Box<dyn Error>>;
}

struct Implementation<TData, TOnchain, TP2p>
where
  TData: data::Service,
  TOnchain: onchain::Service,
  TP2p: p2p::Service,
{
  data: TData,
  onchain: Pin<Box<TOnchain>>,
  p2p: TP2p,
}

pub fn new_service<TData, TOnchain, TP2p>(
  data: TData,
  onchain: TOnchain,
  mut p2p: TP2p,
) -> (impl Service, impl Future<Output = ()>)
where
  TData: data::Service,
  TOnchain: onchain::Service,
  TP2p: p2p::Service,
{
  let onchain_clone = onchain.clone();
  let data_clone = data.clone();
  (
    Implementation {
      data,
      onchain: Box::pin(onchain),
      p2p: p2p.clone(),
    },
    async move {
      while let Some(ev) = p2p.next().await {
        trace!("p2p: {:?}", ev);
        match ev {
          Event::ReceivedLeaseProposal { peer_id, proposal } => {
            let onchain = onchain_clone.clone();
            let data = data_clone.clone();
            let p2p_clone = p2p.clone();
            tokio::task::spawn(async move {
              let result = seal_lease(data, onchain, p2p_clone, peer_id, proposal).await;
              if let Err(e) = result {
                warn!("error while trying to seal deal: {}", e);
              }
            });
          }
        }
      }
    },
  )
}

impl<TData, TOnchain, TP2p> Clone for Implementation<TData, TOnchain, TP2p>
where
  TData: data::Service,
  TOnchain: onchain::Service,
  TP2p: p2p::Service,
{
  fn clone(&self) -> Self {
    Implementation {
      data: self.data.clone(),
      onchain: Box::pin(Clone::clone(self.onchain.deref())),
      p2p: self.p2p.clone(),
    }
  }
}

#[async_trait]
impl<TData, TOnchain, TP2p> Service for Implementation<TData, TOnchain, TP2p>
where
  TData: data::Service,
  TOnchain: onchain::Service,
  TP2p: p2p::Service,
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

    let expiration = terms.proposal_expiration.clone();
    let token_address = terms.token_address.clone();
    self
      .p2p
      .send_proposal(peer_id, nonce, terms, Signature::from(signature), data)
      .await;

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

async fn seal_lease<TData, TOnchain, TP2p>(
  data: TData,
  onchain: TOnchain,
  p2p: TP2p,
  peer_id: PeerId,
  proposal: LeaseProposal,
) -> Result<TransactionResult, Box<dyn Error>>
where
  TData: data::Service,
  TOnchain: onchain::Service,
  TP2p: p2p::Service,
{
  let lessee_address = p2p
    .find_public_key(&peer_id)
    .as_ref()
    .map(IntoAddress::into_address)
    .ok_or("peer id not found")?;
  let data_parameters = data.parameters(proposal.data.as_slice()).await;

  onchain
    .seal_lease(
      lessee_address,
      proposal.nonce,
      proposal.lease_terms,
      data_parameters,
      proposal.signature,
    )
    .await
}
