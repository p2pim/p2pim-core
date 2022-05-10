use crate::cmd::{arg_token, arg_url, ARG_TOKEN, ARG_URL};
use bigdecimal::BigDecimal;
use clap::{Arg, ArgMatches, Command};
use libp2p::PeerId;
use num_bigint::{BigInt, Sign, ToBigInt};
use p2pim::proto::api::p2pim_client::P2pimClient;
use p2pim::proto::api::{GetBalanceRequest, StoreRequest};
use std::convert::TryInto;
use std::str::FromStr;
use std::time::Duration;
use web3::types::H256;

pub const STORE_CMD: &str = "store";

const ARG_DATA_FILE: &str = "data_file";
const ARG_DURATION: &str = "duration";
const ARG_PEER_ID: &str = "peer";
const ARG_PENALTY: &str = "penalty";
const ARG_PRICE: &str = "price";

pub fn command() -> Command<'static> {
  Command::new(STORE_CMD)
    .about("store data in a peer")
    .arg(arg_url())
    .arg(arg_peer_id())
    .arg(arg_token().long(ARG_TOKEN))
    .arg(arg_price())
    .arg(arg_penalty())
    .arg(arg_duration())
    .arg(arg_data_file())
}

fn arg_data_file() -> Arg<'static> {
  Arg::new(ARG_DATA_FILE).takes_value(true).required(true).help("file to store")
}

fn arg_duration() -> Arg<'static> {
  Arg::new(ARG_DURATION)
    .long(ARG_DURATION)
    .takes_value(true)
    .required(true)
    .validator(parse_duration::parse)
    .help("duration of the lease")
}

fn arg_peer_id() -> Arg<'static> {
  Arg::new(ARG_PEER_ID)
    .long(ARG_PEER_ID)
    .takes_value(true)
    .required(true)
    .help("peer where store the data")
}

fn arg_penalty() -> Arg<'static> {
  Arg::new(ARG_PENALTY)
    .long(ARG_PENALTY)
    .takes_value(true)
    .required(true)
    .validator(bigdecimal::BigDecimal::from_str)
    .help("penalty applied to the lessor in case storage lost")
}

fn arg_price() -> Arg<'static> {
  Arg::new(ARG_PRICE)
    .long(ARG_PRICE)
    .takes_value(true)
    .required(true)
    .validator(bigdecimal::BigDecimal::from_str)
    .help("price for the lease")
}

pub fn run(matches: &ArgMatches) -> Result<(), Box<dyn std::error::Error>> {
  let rpc_url = matches.value_of_t(ARG_URL)?;
  let peer_id = matches.value_of_t(ARG_PEER_ID)?;
  let token_addr = matches.value_of_t(ARG_TOKEN)?;
  let price = matches.value_of_t(ARG_PRICE)?;
  let penalty = matches.value_of_t(ARG_PENALTY)?;
  let duration = parse_duration::parse(matches.value_of_t::<String>(ARG_DURATION)?.as_str())?;
  let data_file = matches.value_of_t(ARG_DATA_FILE)?;
  tokio::runtime::Builder::new_multi_thread()
    .enable_all()
    .build()
    .unwrap()
    .block_on(run_store(rpc_url, peer_id, token_addr, price, penalty, duration, data_file))
}

async fn run_store(
  rpc_url: String,
  peer_id: PeerId,
  token_addr: web3::types::Address,
  price: BigDecimal,
  penalty: BigDecimal,
  duration: Duration,
  data_file: String,
) -> Result<(), Box<dyn std::error::Error>> {
  let mut client = P2pimClient::connect(rpc_url).await?;
  let get_balance_request = GetBalanceRequest {
    token_address: Some(token_addr.into()),
  };
  let response = client.get_balance(get_balance_request).await?;
  let decimals = response
    .get_ref()
    .balance
    .as_ref()
    .and_then(|v| v.token.as_ref())
    .map(|v| v.decimals)
    .ok_or("TODO: invalid response")? as i64;

  let abs_price = convert_amount(price, decimals, "price")?;
  let abs_penalty = convert_amount(penalty, decimals, "penalty")?;

  // TODO This does not have any limit or check or anything
  let data = tokio::fs::read(data_file).await?;

  let store_request = StoreRequest {
    peer_id: Some(peer_id.into()),
    token_address: Some(token_addr.into()),
    price: Some(abs_price.try_into()?),
    penalty: Some(abs_penalty.try_into()?),
    lease_duration: Some(prost_types::Duration {
      seconds: duration.as_secs() as i64,
      nanos: 0,
    }),
    data,
  };

  let response = client.store(store_request).await?;
  let hash: H256 = response
    .get_ref()
    .transaction_hash
    .as_ref()
    .ok_or("empty transaction hash")?
    .into();
  println!("store sucessfully, tx hash: 0x{:x}", hash);

  Ok(())
}

fn convert_amount(original: BigDecimal, decimals: i64, name: &str) -> Result<BigInt, Box<dyn std::error::Error>> {
  let abs_amount: BigDecimal = original * BigDecimal::new(1.into(), -decimals);
  if !abs_amount.is_integer() {
    Err(format!("TODO(formatting): the amount for {} has too many decimals", name).into())
  } else if abs_amount.sign() == Sign::Minus {
    Err(format!("TODO:(formatting): the amount for {} cannot be negative", name).into())
  } else {
    Ok(abs_amount.to_bigint().expect("this will never happens").try_into()?)
  }
}
