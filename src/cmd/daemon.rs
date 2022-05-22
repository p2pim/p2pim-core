use std::str::FromStr;

use clap::{Arg, ArgMatches, Command};
use p2pim::daemon::DaemonOpts;
use typed_arena::Arena;

pub const CMD_NAME: &str = "daemon";

const ARG_ETH_URL: &str = "eth.url";
const ARG_ETH_MASTER: &str = "eth.master";

const ARG_RPC_ADDRESS: &str = "rpc.address";
const ARG_RPC_ADDRESS_DEFAULT: &str = "127.0.0.1:8122";

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

fn arg_s3_address<'a>() -> Arg<'a> {
  Arg::new(ARG_S3_ADDRESS)
    .long(ARG_S3_ADDRESS)
    .takes_value(true)
    .value_name("ADDRESS")
    .default_value(ARG_S3_ADDRESS_DEFAULT)
    .help("s3 server listening address")
}

pub fn command(buf: &mut Arena<String>) -> Command {
  Command::new("daemon")
    .about("run daemon")
    .arg(arg_eth_url(buf))
    .arg(arg_eth_master())
    .arg(arg_rpc_address())
    .arg(arg_s3())
    .arg(arg_s3_address())
}

pub fn run(matches: &ArgMatches) -> Result<(), Box<dyn std::error::Error>> {
  let daemon_opts = DaemonOpts {
    eth_addr: matches.value_of_t(ARG_ETH_URL)?,
    rpc_addr: matches.value_of_t(ARG_RPC_ADDRESS)?,
    master_addr: matches
      .value_of(ARG_ETH_MASTER)
      .map(web3::types::Address::from_str)
      .transpose()?,
    s3_addr: if matches.is_present(ARG_S3) {
      Some(matches.value_of_t(ARG_S3_ADDRESS)?)
    } else {
      None
    },
  };
  tokio::runtime::Builder::new_multi_thread()
    .enable_all()
    .build()
    .unwrap()
    .block_on(p2pim::daemon::listen_and_serve(daemon_opts))
}
