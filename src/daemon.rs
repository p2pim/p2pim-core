use crate::lessor::Ask;
use crate::onchain::Service;
use crate::types::TokenMetadata;
use crate::{onchain, p2p};
use bigdecimal::BigDecimal;
use futures::future::try_join_all;
use libp2p::identity::{secp256k1, Keypair};
use log::info;
use num_bigint::{Sign, ToBigInt};
use std::collections::HashMap;
use std::error::Error;
use std::future::Future;
use std::net::SocketAddr;
use std::ops::Range;
use std::pin::Pin;
use std::time::Duration;
use url::Url;
use web3::types::{Address, U256};

pub struct DaemonOpts {
  pub rpc_addr: SocketAddr,
  pub eth_opts: EthOpts,
  pub lessor_opts: LessorOpts,
  pub mdns_opts: MdnsOpts,
  pub s3_opts: S3Opts,
}

pub struct LessorOpts {
  pub token_lease_terms: HashMap<Address, TokenLeaseAsk>,
}

pub struct TokenLeaseAsk {
  pub duration_range: Range<Duration>,
  pub size_range: Range<usize>,
  pub min_tokens_total: BigDecimal,
  pub min_tokens_gb_hour: BigDecimal,
  pub max_penalty_rate: f32,
}

pub struct EthOpts {
  pub url: Url,
  pub master_addr: Option<Address>,
}

pub struct S3Opts {
  pub enabled: bool,
  pub s3_addr: SocketAddr,
}

pub struct MdnsOpts {
  pub enabled: bool,
}

pub async fn listen_and_serve(opts: &DaemonOpts) -> Result<(), Box<dyn std::error::Error>> {
  info!("initializing p2pim");

  let secp256k1_keypair = secp256k1::Keypair::generate();
  let keypair = Keypair::Secp256k1(secp256k1_keypair.clone());
  let p2p = p2p::create_p2p(keypair, opts.mdns_opts.enabled).await?;

  type ServeFuture = Pin<Box<dyn Future<Output = Result<(), Box<dyn Error>>>>>;

  let cryptography = crate::cryptography::new_service();
  let data = crate::data::new_service(
    cryptography,
    dirs::home_dir()
      .map(|v| {
        let mut new_path = v;
        new_path.push(".p2pim");
        new_path.push("datastore");
        new_path
      })
      .expect("no home dir found"),
  );
  let private_key_raw = secp256k1_keypair.secret().to_bytes();

  let onchain = crate::onchain::new_service(onchain::OnchainParams {
    eth_url: opts.eth_opts.url.clone(),
    private_key: private_key_raw,
    master_address: opts.eth_opts.master_addr,
  })
  .await?;

  let persistence = crate::persistence::new_service();

  let deployed_map: HashMap<Address, Option<TokenMetadata>> = onchain.deployed_tokens().await.into_iter().collect();

  let asks = opts
    .lessor_opts
    .token_lease_terms
    .iter()
    .map(|(token_address, opts)| {
      deployed_map
        .get(token_address)
        .map(|v| {
          v.clone()
            .ok_or_else::<Box<dyn Error>, _>(|| "TODO: Token with no metadata".into())
        })
        .unwrap_or_else(|| Err("TODO: Token not deployed".into()))
        .and_then(|v| {
          Ok((
            token_address.clone(),
            Ask {
              duration_range: opts.duration_range.clone(),
              size_range: opts.size_range.clone(),
              max_penalty_rate: opts.max_penalty_rate,
              min_tokens_total: convert_bigdecimal(opts.min_tokens_total.clone(), v.decimals)?,
              min_tokens_gb_hour: convert_bigdecimal(opts.min_tokens_gb_hour.clone(), v.decimals)?,
            },
          ))
        })
    })
    .collect::<Result<Vec<(Address, Ask)>, _>>()?;

  let lessor = crate::lessor::new_service(asks);

  let (reactor, reactor_fut) = crate::reactor::new_service(data, lessor, onchain.clone(), p2p.clone(), persistence.clone());

  let grpc: ServeFuture = Box::pin(crate::grpc::listen_and_serve(
    opts.rpc_addr,
    onchain.clone(),
    p2p.clone(),
    reactor.clone(),
    persistence.clone(),
  ));

  let s3: Option<ServeFuture> = opts
    .s3_opts
    .enabled
    .then(|| Box::pin(crate::s3::listen_and_serve(opts.s3_opts.s3_addr)) as ServeFuture);
  let reactor_fut2: ServeFuture = Box::pin(futures::FutureExt::map(reactor_fut, Result::Ok));
  let futures: Vec<ServeFuture> = vec![Some(reactor_fut2), Some(grpc), s3].into_iter().flatten().collect();
  try_join_all(futures).await.map(|_| ())
}

fn convert_bigdecimal(amount: BigDecimal, decimals: u8) -> Result<U256, Box<dyn Error>> {
  let abs_amount = amount * BigDecimal::new(1.into(), -(decimals as i64));
  if !abs_amount.is_integer() {
    Err("TODO(formatting): the amount has too many decimals".into())
  } else if abs_amount.sign() == Sign::Minus {
    Err("TODO:(formatting): the amount cannot be negative".into())
  } else {
    let int_value = abs_amount.to_bigint().expect("checked already if it is integer");
    let bytes = int_value.to_bytes_le().1;
    Ok(web3::types::U256::from_little_endian(bytes.as_slice()))
  }
}
