use crate::types::{DataParameters, LeaseTerms, Signature};
use crate::utils::ethereum::IntoAddress;
use ethcontract::errors::EventError;
use ethcontract::transaction::TransactionResult;
use ethcontract::{Bytes, Event, EventStatus};
use futures::stream::SelectAll;
use futures::{select, Stream, StreamExt};
use log::{debug, error, trace, warn};
use p2pim_ethereum_contracts::third::openzeppelin;
use p2pim_ethereum_contracts::{P2pimAdjudicator, P2pimMasterRecord};
use secp256k1::Secp256k1;
use std::borrow::Borrow;
use std::collections::HashMap;
use std::convert::TryInto;
use std::error::Error;
use std::pin::Pin;
use std::time;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tonic::async_trait;
use web3::ethabi::{Token, Topic};
use web3::signing::{Key, SecretKeyRef};
use web3::types::{Address, Block, BlockId, H256};
use web3::{DuplexTransport, Transport};

#[derive(Clone)]
pub struct OnchainParams {
  pub private_key: [u8; 32],
  pub master_address: Option<Address>,
}

#[async_trait]
pub trait Service: Clone + Send + Sync + 'static {
  type StreamType: Stream<Item = Result<Event<EventStatus<p2pim_ethereum_contracts::adjudicator::event_data::LeaseSealed>>, EventError>>
    + Unpin;

  async fn block(&self, block_id: BlockId) -> Result<Option<Block<H256>>, Box<dyn Error>>;

  async fn listen_adjudicator_events(&self) -> Self::StreamType;

  fn own_address(&self) -> web3::types::Address;

  async fn seal_lease(
    &self,
    lessee_address: Address,
    nonce: u64,
    terms: LeaseTerms,
    data_parameters: DataParameters,
    lessee_signature: Signature,
  ) -> Result<TransactionResult, Box<dyn Error>>;

  async fn sign_proposal(
    &self,
    lessor_address: &Address,
    nonce: u64,
    terms: &LeaseTerms,
    data_parameters: &DataParameters,
  ) -> Signature;

  async fn wait_for_seal_lease(
    &self,
    token_address: &Address,
    lessor_address: Address,
    nonce: u64,
    until: SystemTime,
  ) -> Result<
    Option<ethcontract::Event<EventStatus<p2pim_ethereum_contracts::adjudicator::event_data::LeaseSealed>>>,
    Box<dyn Error>,
  >;
}

#[derive(Clone)]
struct Implementation<T>
where
  T: Transport + DuplexTransport,
{
  params: OnchainParams,
  web3: web3::Web3<T>,
  deployments: HashMap<Address, (openzeppelin::IERC20Metadata, P2pimAdjudicator)>,
}

pub async fn new_service<B, T>(params: OnchainParams, web3: web3::Web3<T>) -> Result<impl Service, Box<dyn Error>>
where
  B: std::future::Future<Output = web3::Result<Vec<web3::Result<serde_json::Value>>>> + Send + 'static,
  T: web3::Transport + web3::BatchTransport<Batch = B> + DuplexTransport + Send + Sync + 'static,
  <T as DuplexTransport>::NotificationStream: std::marker::Send + Unpin,
  <T as Transport>::Out: std::future::Future<Output = web3::Result<serde_json::Value>> + Send + 'static,
{
  debug!("initializing master record contract");
  let instance = if let Some(addr) = params.master_address {
    Ok(P2pimMasterRecord::at(&web3, addr))
  } else {
    P2pimMasterRecord::deployed(&web3).await
  }?;
  debug!("using master record contract on address {}", instance.address());

  debug!("reading accounts");
  let accounts = web3.eth().accounts().await?;
  let account_wallet = accounts.get(0).map(Clone::clone).ok_or("no accounts configured")?;
  debug!("using account for wallet {:?}", account_wallet);

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
        token,
        (
          openzeppelin::IERC20Metadata::at(&web3, token),
          P2pimAdjudicator::at(&web3, adjudicator_addr),
        ),
      )
    })
    .collect();
  debug!("found deployments {:?}", deployments);

  Ok(Implementation {
    params,
    web3,
    deployments,
  })
}

impl<F, T> Implementation<T>
where
  F: std::future::Future<Output = web3::Result<serde_json::Value>> + Send + 'static,
  T: Transport<Out = F> + DuplexTransport + Send + Sync + 'static,
{
  async fn sign(
    &self,
    lessee_address: &Address,
    lessor_address: &Address,
    nonce: u64,
    terms: &LeaseTerms,
    data_parameters: &DataParameters,
  ) -> Signature {
    let message = [
      Token::Address(terms.token_address),
      Token::Address(lessee_address.clone()),
      Token::Address(lessor_address.clone()),
      Token::Uint(nonce.into()),
      Token::FixedBytes(data_parameters.merkle_root.clone()),
      Token::Uint(data_parameters.size.into()),
      Token::Uint(terms.price),
      Token::Uint(terms.penalty),
      Token::Uint(terms.lease_duration.as_secs().into()),
      Token::Uint(
        terms
          .proposal_expiration
          .duration_since(time::UNIX_EPOCH)
          .unwrap()
          .as_secs()
          .into(),
      ),
    ];
    let abi_encoded = web3::ethabi::encode(&message);
    let message_hash = web3::signing::keccak256(abi_encoded.as_slice());

    trace!(
      "message {}, hash to sign {}, lesse: {}, lessor: {}",
      hex::encode(abi_encoded.as_slice()),
      hex::encode(message_hash),
      lessee_address,
      lessor_address
    );
    let secret = secp256k1::SecretKey::from_slice(self.params.private_key.as_slice()).expect("this will never happen");

    Signature::from(
      SecretKeyRef::new(&secret)
        .sign(web3::signing::keccak256(abi_encoded.as_slice()).as_slice(), None)
        .expect("Why can fail?"),
    )
  }
}

#[async_trait]
impl<F, N, T> Service for Implementation<T>
where
  F: std::future::Future<Output = web3::Result<serde_json::Value>> + Send + 'static,
  N: futures::Stream + Send + Unpin,
  T: Transport<Out = F> + DuplexTransport<NotificationStream = N> + Send + Sync + 'static,
{
  type StreamType = SelectAll<
    Pin<
      Box<
        dyn Stream<
          Item = Result<Event<EventStatus<p2pim_ethereum_contracts::adjudicator::event_data::LeaseSealed>>, EventError>,
        >,
      >,
    >,
  >;

  async fn block(&self, block_id: BlockId) -> Result<Option<Block<H256>>, Box<dyn Error>> {
    Ok(self.web3.eth().block(block_id).await?)
  }

  async fn listen_adjudicator_events(&self) -> Self::StreamType {
    let self_address = self.own_address();

    fn event_stream(
      adjudicator: &P2pimAdjudicator,
      lessor_address: Option<Address>,
      lessee_address: Option<Address>,
    ) -> Pin<
      Box<
        dyn Stream<
          Item = Result<Event<EventStatus<p2pim_ethereum_contracts::adjudicator::event_data::LeaseSealed>>, EventError>,
        >,
      >,
    > {
      Box::pin(
        adjudicator
          .clone()
          .events()
          .lease_sealed()
          .lessor(lessor_address.map(Topic::This).unwrap_or(Topic::Any))
          .lessee(lessee_address.map(Topic::This).unwrap_or(Topic::Any))
          .stream(),
      )
    }

    let streams = self.deployments.values().flat_map(|(_, adjudicator)| {
      vec![
        event_stream(adjudicator, Some(self_address), None),
        event_stream(adjudicator, None, Some(self_address)),
      ]
    });

    futures::stream::select_all(streams)
  }

  fn own_address(&self) -> Address {
    let context = Secp256k1::new();
    let secret = secp256k1::SecretKey::from_slice(self.params.private_key.as_slice()).expect("this will never happen");
    let public_key = secp256k1::PublicKey::from_secret_key(&context, &secret);
    public_key.borrow().into_address()
  }

  async fn seal_lease(
    &self,
    lessee_address: Address,
    nonce: u64,
    terms: LeaseTerms,
    data_parameters: DataParameters,
    lessee_signature: Signature,
  ) -> Result<TransactionResult, Box<dyn Error>> {
    let lessor_address = self.own_address();

    let lessor_signature = self
      .sign(&lessee_address, &lessor_address, nonce, &terms, &data_parameters)
      .await;

    let merkle_root: [u8; 32] = data_parameters
      .merkle_root
      .clone()
      .try_into()
      .expect("TODO this should never happen");

    let (_, adjudicator) = self.deployments.get(&terms.token_address).ok_or("deployment not found")?;
    let lease_deal = (
      lessee_address.clone(),
      lessor_address.clone(),
      nonce,
      Bytes(merkle_root),
      data_parameters.size as u64,
      terms.price,
      terms.penalty,
      terms.lease_duration.as_secs().into(),
      terms
        .proposal_expiration
        .duration_since(UNIX_EPOCH)
        .expect("TODO: this should not happen")
        .as_secs()
        .into(),
    );
    let result = adjudicator
      .seal_lease(
        lease_deal,
        Bytes(lessee_signature.serialize()),
        Bytes(lessor_signature.serialize()),
      )
      .send()
      .await?;
    Ok(result)
  }

  async fn sign_proposal(
    &self,
    lessor_address: &Address,
    nonce: u64,
    terms: &LeaseTerms,
    data_parameters: &DataParameters,
  ) -> Signature {
    let lessee_address = &self.own_address();
    self.sign(lessee_address, lessor_address, nonce, terms, data_parameters).await
  }

  async fn wait_for_seal_lease(
    &self,
    token_address: &Address,
    lessor_address: Address,
    nonce: u64,
    until: SystemTime,
  ) -> Result<
    Option<ethcontract::Event<EventStatus<p2pim_ethereum_contracts::adjudicator::event_data::LeaseSealed>>>,
    Box<dyn Error>,
  > {
    let (_, adjudicator) = self.deployments.get(token_address).ok_or("adjudicator not found")?;
    let lessee_address = self.own_address();
    let last_block = self.web3.eth().block_number().await?;
    // TODO This is using polling, maybe better to use subscriptions
    let mut event_stream = Box::pin(
      adjudicator
        .events()
        .lease_sealed()
        .from_block(ethcontract::BlockNumber::Number(
          last_block.checked_sub(10u64.into()).unwrap_or(0u64.into()),
        ))
        .lessor(Topic::This(lessor_address))
        .lessee(Topic::This(lessee_address))
        .poll_interval(Duration::from_secs(1))
        .stream()
        .fuse(),
    );

    let mut new_heads = self.web3.eth_subscribe().subscribe_new_heads().await?.fuse();

    // TODO Refactor
    let result = {
      let mut r = None;
      loop {
        select! {
          ev = event_stream.next() => match ev {
            Some(Ok(e)) => {
              if e.inner_data().nonce == nonce {
                r = Some(Ok(Some(e)))
              }
            },
            Some(Err(e)) => warn!("error in event stream: {}", e),
            None => {
              warn!("the stream should never be closed");
              r = Some(Err("event stream closed unexpected".to_string()));
            }
          },
          head = new_heads.next() => match head {
            Some(Ok(h)) => if UNIX_EPOCH + Duration::from_secs(h.timestamp.as_u64()) > until {
              r = Some(Ok(None));
            },
            Some(Err(e)) => warn!("error in heads stream: {}", e),
            None => {
              warn!("head stream closed unexpected");
              r = Some(Err("head stream closed unexpected".to_string()));
            }
          }
        }
        if r.is_some() {
          break;
        }
      }
      r.unwrap()
    };

    match new_heads.into_inner().unsubscribe().await {
      Ok(true) => trace!("unsubscribed from heads"),
      Ok(false) => warn!("unsubscribed returns false"),
      Err(e) => error!("error while unsubscribe from heads: {}", e),
    };
    Ok(result?)
  }
}
