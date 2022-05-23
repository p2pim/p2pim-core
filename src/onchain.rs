use crate::types::{Balance, DataParameters, LeaseTerms, Signature, StorageBalance, TokenMetadata, WalletBalance};
use crate::utils::ethereum::IntoAddress;
use ethcontract::errors::{EventError, MethodError};
use ethcontract::transaction::TransactionResult;
use ethcontract::{Account, Bytes, Event, EventStatus, PrivateKey};
use futures::stream::SelectAll;
use futures::{select, Stream, StreamExt};
use log::{debug, error, info, trace, warn};
use p2pim_ethereum_contracts::third::openzeppelin;
use p2pim_ethereum_contracts::{P2pimAdjudicator, P2pimMasterRecord};
use secp256k1::Secp256k1;
use std::borrow::Borrow;
use std::collections::HashMap;
use std::convert::TryInto;
use std::fmt::{Debug, Display, Formatter};
use std::pin::Pin;
use std::time;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tonic::async_trait;
use url::Url;
use web3::ethabi::{Token, Topic};
use web3::signing::{Key, SecretKeyRef};
use web3::transports::{Either, Ipc, WebSocket};
use web3::types::{Address, Block, BlockId, H256, U256};

#[derive(Clone)]
pub struct OnchainParams {
  pub eth_url: Url,
  // TODO Review this as could be dangerous to keep this in memory
  pub private_key: [u8; 32],
  pub master_address: Option<Address>,
}

#[derive(Debug)]
pub enum Error {
  TokenNotDeployed(Address),
  MethodError(MethodError),
  Web3Error(web3::error::Error),
}

pub type Result<T> = core::result::Result<T, Error>;

impl Display for Error {
  fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
    match self {
      Error::TokenNotDeployed(_) => f.write_str("token not deployed"),
      Error::MethodError(err) => std::fmt::Display::fmt(err, f),
      Error::Web3Error(err) => std::fmt::Display::fmt(err, f),
    }
  }
}

impl std::error::Error for Error {
  fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
    match self {
      Error::TokenNotDeployed(_) => None,
      Error::MethodError(err) => Some(err),
      Error::Web3Error(err) => Some(err),
    }
  }
}

impl From<MethodError> for Error {
  fn from(value: MethodError) -> Self {
    Error::MethodError(value)
  }
}

impl From<web3::error::Error> for Error {
  fn from(value: web3::Error) -> Self {
    Error::Web3Error(value)
  }
}

// TODO Better error handling, not returning dyn Error
#[async_trait]
pub trait Service: Clone + Send + Sync + 'static {
  type StreamType: Stream<
      Item = core::result::Result<
        Event<EventStatus<p2pim_ethereum_contracts::adjudicator::event_data::LeaseSealed>>,
        EventError,
      >,
    > + Unpin;

  async fn block(&self, block_id: BlockId) -> Result<Option<Block<H256>>>;

  async fn listen_adjudicator_events(&self) -> Self::StreamType;

  fn account_wallet(&self) -> web3::types::Address;
  fn account_storage(&self) -> web3::types::Address;

  async fn seal_lease(
    &self,
    lessee_address: Address,
    nonce: u64,
    terms: LeaseTerms,
    data_parameters: DataParameters,
    lessee_signature: Signature,
  ) -> Result<TransactionResult>;

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
  ) -> Result<Option<ethcontract::Event<EventStatus<p2pim_ethereum_contracts::adjudicator::event_data::LeaseSealed>>>>;

  async fn deployed_tokens(&self) -> Vec<(Address, Option<TokenMetadata>)>;
  async fn balance(&self, token_address: &Address) -> Result<Balance>;

  async fn withdraw(&self, token_address: &Address, amount: U256) -> Result<TransactionResult>;
  async fn deposit(&self, token_address: &Address, amount: U256) -> Result<TransactionResult>;

  async fn approve(&self, token_address: &Address) -> Result<TransactionResult>;
}

#[derive(Clone)]
struct Implementation {
  account_wallet: Address,
  account_storage: Address,
  params: OnchainParams,
  private_key: ethcontract::PrivateKey,
  web3: web3::Web3<Either<WebSocket, Ipc>>,
  deployments: HashMap<Address, (openzeppelin::IERC20Metadata, P2pimAdjudicator)>,
}

pub async fn new_service(params: OnchainParams) -> core::result::Result<impl Service, Box<dyn std::error::Error>> {
  info!("initializing onchain subsystem");

  debug!("creating transport using {}", params.eth_url);
  let transport = match params.eth_url.scheme() {
    "file" => Ok(Either::Right(web3::transports::ipc::Ipc::new(params.eth_url.path()).await?)),
    "ws" | "wss" => Ok(Either::Left(
      web3::transports::ws::WebSocket::new(params.eth_url.as_str()).await?,
    )),
    unsupported => Err(format!("unsupported schema: {}", unsupported)),
  }?;

  debug!("creating web3");
  let web3 = web3::Web3::new(transport);

  let network_id = web3.net().version().await?;
  info!("connected to eth network with id {}", network_id);

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

  let context = Secp256k1::new();
  let secret = secp256k1::SecretKey::from_slice(params.private_key.as_slice()).expect("this will never happen");
  let public_key = secp256k1::PublicKey::from_secret_key(&context, &secret);
  let account_storage = public_key.borrow().into_address();
  let private = PrivateKey::from_raw(params.private_key).expect("TODO: this should not happen");

  Ok(Implementation {
    account_wallet,
    account_storage,
    params,
    private_key: private,
    web3,
    deployments,
  })
}

impl Implementation {
  fn deployment(&self, address: &Address) -> Result<(openzeppelin::IERC20Metadata, P2pimAdjudicator)> {
    self
      .deployments
      .get(address)
      .cloned()
      .ok_or_else(|| Error::TokenNotDeployed(*address))
  }

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
      Token::Address(*lessee_address),
      Token::Address(*lessor_address),
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
    let eth_message_hash = web3::signing::hash_message(message_hash);

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
        .sign(eth_message_hash.as_bytes(), None)
        .expect("Why can fail?"),
    )
  }
}

#[async_trait]
impl Service for Implementation {
  type StreamType = SelectAll<
    Pin<
      Box<
        dyn Stream<
          Item = core::result::Result<
            Event<EventStatus<p2pim_ethereum_contracts::adjudicator::event_data::LeaseSealed>>,
            EventError,
          >,
        >,
      >,
    >,
  >;

  async fn block(&self, block_id: BlockId) -> Result<Option<Block<H256>>> {
    Ok(self.web3.eth().block(block_id).await?)
  }

  async fn listen_adjudicator_events(&self) -> Self::StreamType {
    let self_address = self.account_storage();

    fn event_stream(
      adjudicator: &P2pimAdjudicator,
      lessor_address: Option<Address>,
      lessee_address: Option<Address>,
    ) -> Pin<
      Box<
        dyn Stream<
          Item = core::result::Result<
            Event<EventStatus<p2pim_ethereum_contracts::adjudicator::event_data::LeaseSealed>>,
            EventError,
          >,
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

  fn account_wallet(&self) -> Address {
    self.account_wallet
  }

  fn account_storage(&self) -> Address {
    self.account_storage
  }

  async fn seal_lease(
    &self,
    lessee_address: Address,
    nonce: u64,
    terms: LeaseTerms,
    data_parameters: DataParameters,
    lessee_signature: Signature,
  ) -> Result<TransactionResult> {
    let lessor_address = self.account_storage();

    let lessor_signature = self
      .sign(&lessee_address, &lessor_address, nonce, &terms, &data_parameters)
      .await;

    let merkle_root: [u8; 32] = data_parameters
      .merkle_root
      .clone()
      .try_into()
      .expect("TODO this should never happen");

    let (_, adjudicator) = self.deployment(&terms.token_address)?;
    let lease_deal = (
      lessee_address,
      lessor_address,
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
    let lessee_address = &self.account_storage();
    self.sign(lessee_address, lessor_address, nonce, terms, data_parameters).await
  }

  async fn wait_for_seal_lease(
    &self,
    token_address: &Address,
    lessor_address: Address,
    nonce: u64,
    until: SystemTime,
  ) -> Result<Option<ethcontract::Event<EventStatus<p2pim_ethereum_contracts::adjudicator::event_data::LeaseSealed>>>> {
    let (_, adjudicator) = self.deployment(token_address)?;
    let lessee_address = self.account_storage();
    let last_block = self.web3.eth().block_number().await?;
    // TODO This is using polling, maybe better to use subscriptions
    let mut event_stream = Box::pin(
      adjudicator
        .events()
        .lease_sealed()
        .from_block(ethcontract::BlockNumber::Number(
          last_block.checked_sub(10u64.into()).unwrap_or_default(),
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
                r = Some(Some(e))
              }
            },
            Some(Err(e)) => warn!("TODO: error in event stream: {}", e),
            None => unreachable!("TODO: the stream should never be closed"),
          },
          head = new_heads.next() => match head {
            Some(Ok(h)) => if UNIX_EPOCH + Duration::from_secs(h.timestamp.as_u64()) > until {
              r = Some(None);
            },
            Some(Err(e)) => warn!("TODO: error in heads stream: {}", e),
            None => unreachable!("TODO: the stream should never be closed"),
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
    Ok(result)
  }

  async fn deployed_tokens(&self) -> Vec<(Address, Option<TokenMetadata>)> {
    futures::stream::iter(&self.deployments)
      .then(|(address, (token, _))| async move { (*address, read_metadata(token).await) })
      .collect()
      .await
  }

  async fn balance(&self, token_address: &Address) -> Result<Balance> {
    let (token, adjudicator) = self.deployment(token_address)?;
    let (available_p2pim, locked_rents, locked_lets) = adjudicator.balance(self.account_storage).call().await?;

    let available_account = token.balance_of(self.account_wallet).call().await?;
    let allowance_account = token.allowance(self.account_wallet, adjudicator.address()).call().await?;

    let token_metadata = read_metadata(&token).await;

    Ok(Balance {
      token_metadata,
      storage_balance: StorageBalance {
        available: available_p2pim,
        locked_rents,
        locked_lets,
      },
      wallet_balance: WalletBalance {
        available: available_account,
        allowance: allowance_account,
      },
    })
  }

  async fn withdraw(&self, token_addres: &Address, amount: U256) -> Result<TransactionResult> {
    let (_, adjudicator) = self.deployment(token_addres)?;
    Ok(
      adjudicator
        .methods()
        .withdraw(amount, self.account_wallet)
        .from(Account::Offline(self.private_key.clone(), None)) // TODO should we use the chain id?
        .send()
        .await?,
    )
  }

  async fn deposit(&self, token_addres: &Address, amount: U256) -> Result<TransactionResult> {
    let (_, adjudicator) = self.deployment(token_addres)?;
    Ok(adjudicator.methods().deposit(amount, self.account_storage).send().await?)
  }

  async fn approve(&self, token_address: &Address) -> Result<TransactionResult> {
    let (token, adjudicator) = self.deployment(token_address)?;
    Ok(
      token
        .approve(adjudicator.address(), U256::max_value())
        .confirmations(0)
        .send()
        .await?,
    )
  }
}

fn ok_or_warn<R, E: std::fmt::Display>(
  result: core::result::Result<R, E>,
  method: &str,
  address: web3::types::Address,
) -> Option<R> {
  if let Err(e) = result.as_ref() {
    warn!("error calling `{}` method error={} address={}", method, e, address)
  }
  result.ok()
}

async fn read_metadata(token: &openzeppelin::IERC20Metadata) -> Option<TokenMetadata> {
  let maybe_name = ok_or_warn(token.name().call().await, "name", token.address());
  let maybe_symbol = ok_or_warn(token.methods().symbol().call().await, "symbol", token.address());
  let maybe_decimals = ok_or_warn(token.methods().decimals().call().await, "decimals", token.address());

  match (maybe_name, maybe_symbol, maybe_decimals) {
    (Some(name), Some(symbol), Some(decimals)) => Some(TokenMetadata { name, symbol, decimals }),
    _ => None,
  }
}
