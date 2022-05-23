use bigdecimal::BigDecimal;
use std::collections::HashMap;
use std::ops::Range;
use std::str::FromStr;

use clap::{Arg, ArgMatches, Command};
use p2pim::daemon::{DaemonOpts, EthOpts, LessorOpts, MdnsOpts, S3Opts, TokenLeaseAsk};
use typed_arena::Arena;

pub const CMD_NAME: &str = "daemon";

const ARG_ETH_URL: &str = "eth.url";
const ARG_ETH_MASTER: &str = "eth.master";

const ARG_RPC_ADDRESS: &str = "rpc.address";
const ARG_RPC_ADDRESS_DEFAULT: &str = "127.0.0.1:8122";

const ARG_LESSOR_ASK: &str = "lessor.ask";

const ARG_MDNS: &str = "mdns";

const ARG_S3: &str = "s3";

const ARG_S3_ADDRESS: &str = "s3.address";
const ARG_S3_ADDRESS_DEFAULT: &str = "127.0.0.1:8123";

fn arg_eth_url(buf: &mut Arena<String>) -> Arg {
  let default_value = buf.alloc(format!(
    "file://{}/.ethereum/geth.ipc",
    dirs::home_dir().expect("TODO").to_str().expect("TODO")
  ));
  Arg::new(ARG_ETH_URL)
    .long(ARG_ETH_URL)
    .takes_value(true)
    .value_name("ADDRESS")
    .default_value(default_value)
    .help("ethereum JSON-RPC address")
}

fn arg_eth_master<'a>() -> Arg<'a> {
  Arg::new(ARG_ETH_MASTER)
    .long(ARG_ETH_MASTER)
    .takes_value(true)
    .value_name("ADDRESS")
    .validator(web3::types::Address::from_str)
    .required(false)
    .help("ethereum address of the master record contract")
}

fn arg_rpc_address<'a>() -> Arg<'a> {
  Arg::new(ARG_RPC_ADDRESS)
    .long(ARG_RPC_ADDRESS)
    .takes_value(true)
    .value_name("ADDRESS")
    .default_value(ARG_RPC_ADDRESS_DEFAULT)
    .help("gRPC server listening address")
}

fn arg_s3<'a>() -> Arg<'a> {
  Arg::new(ARG_S3)
    .long(ARG_S3)
    .required(false)
    .takes_value(false)
    .help("Enable the S3 compatible server")
}

fn arg_mdns<'a>() -> Arg<'a> {
  Arg::new(ARG_MDNS)
    .long(ARG_MDNS)
    .required(false)
    .takes_value(false)
    .help("Enable bootstraping using mdns")
}

fn arg_s3_address<'a>() -> Arg<'a> {
  Arg::new(ARG_S3_ADDRESS)
    .long(ARG_S3_ADDRESS)
    .takes_value(true)
    .value_name("ADDRESS")
    .default_value(ARG_S3_ADDRESS_DEFAULT)
    .help("s3 server listening address")
}

fn arg_lessor_ask<'a>() -> Arg<'a> {
  Arg::new(ARG_LESSOR_ASK)
    .long(ARG_LESSOR_ASK)
    .takes_value(true)
    .value_name("TERMS")
    .multiple_occurrences(true)
    .help("lease ask in form TOKEN:min_duration:max_duration:min_size:max_size:min_tokens_total:min_tokens_gb_hour:max_penalty_rate")
}

pub fn command(buf: &mut Arena<String>) -> Command {
  Command::new("daemon")
    .about("run daemon")
    .arg(arg_eth_url(buf))
    .arg(arg_eth_master())
    .arg(arg_rpc_address())
    .arg(arg_s3())
    .arg(arg_s3_address())
    .arg(arg_lessor_ask())
    .arg(arg_mdns())
}

pub fn run(matches: &ArgMatches) -> Result<(), Box<dyn std::error::Error>> {
  let daemon_opts = DaemonOpts {
    rpc_addr: matches.value_of_t(ARG_RPC_ADDRESS)?,
    eth_opts: EthOpts {
      master_addr: matches
        .value_of(ARG_ETH_MASTER)
        .map(web3::types::Address::from_str)
        .transpose()?,
      url: matches.value_of_t(ARG_ETH_URL)?,
    },
    lessor_opts: LessorOpts {
      token_lease_terms: matches
        .values_of(ARG_LESSOR_ASK)
        .map(|values| {
          values
            .map(parse_lessor_ask)
            .collect::<Result<HashMap<web3::types::Address, TokenLeaseAsk>, Box<dyn std::error::Error>>>()
        })
        .unwrap_or_else(|| Ok(Default::default()))?,
    },
    mdns_opts: MdnsOpts {
      enabled: matches.is_present(ARG_MDNS),
    },
    s3_opts: S3Opts {
      enabled: matches.is_present(ARG_S3),
      s3_addr: matches.value_of_t(ARG_S3_ADDRESS)?,
    },
  };
  tokio::runtime::Builder::new_multi_thread()
    .enable_all()
    .build()
    .unwrap()
    .block_on(p2pim::daemon::listen_and_serve(&daemon_opts))
}

pub fn parse_lessor_ask(terms: &str) -> Result<(web3::types::Address, TokenLeaseAsk), Box<dyn std::error::Error>> {
  let parts = terms.split(':').collect::<Vec<_>>();
  if parts.len() != 8 {
    return Err(format!("invalid ask format: required 8 fields, found {}", parts.len()).into());
  }

  //TOKEN:min_duration:max_duration:min_size:max_size:min_tokens_total:min_tokens_gb_hour:max_penalty_rate
  let token = web3::types::Address::from_str(parts.get(0).unwrap())?;
  let min_duration = parse_duration::parse(parts.get(1).unwrap())?;
  let max_duration = parse_duration::parse(parts.get(2).unwrap())?;
  let min_size = humanize_rs::bytes::Bytes::from_str(parts.get(3).unwrap())?;
  let max_size = humanize_rs::bytes::Bytes::from_str(parts.get(4).unwrap())?;
  let min_tokens_total = BigDecimal::from_str(parts.get(5).unwrap())?;
  let min_tokens_gb_hour = BigDecimal::from_str(parts.get(6).unwrap())?;
  let max_penalty_rate = f32::from_str(parts.get(7).unwrap())?;

  if min_duration >= max_duration {
    return Err(
      format!(
        "invalid ask values: min_duration ({}) is greather or equal to max_duration ({})",
        parts.get(1).unwrap(),
        parts.get(2).unwrap()
      )
      .into(),
    );
  }

  if min_size.size() >= max_size.size() {
    return Err(
      format!(
        "invalid ask values: min_size ({}) is greather of equal to max_size ({})",
        parts.get(3).unwrap(),
        parts.get(4).unwrap()
      )
      .into(),
    );
  }

  Ok((
    token,
    TokenLeaseAsk {
      duration_range: Range {
        start: min_duration,
        end: max_duration,
      },
      size_range: Range {
        start: min_size.size(),
        end: max_size.size(),
      },
      min_tokens_total,
      min_tokens_gb_hour,
      max_penalty_rate,
    },
  ))
}
