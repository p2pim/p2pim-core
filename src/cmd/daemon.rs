use std::str::FromStr;

use clap::{Arg, ArgMatches, Command};
use p2pim::daemon::DaemonOpts;

const ARG_ETH_ADDRESS: &str = "eth.address";

const ARG_RPC_ADDRESS: &str = "rpc.address";
const ARG_RPC_ADDRESS_DEFAULT: &str = "127.0.0.1:8122";

const ARG_ETH_MASTER: &str = "eth.master";

fn arg_rpc_address() -> Arg<'static> {
  Arg::new(ARG_RPC_ADDRESS)
    .long(ARG_RPC_ADDRESS)
    .takes_value(true)
    .value_name("ADDRESS")
    .default_value(ARG_RPC_ADDRESS_DEFAULT)
    .help("gRPC server listening address")
}

pub fn command() -> Command<'static> {
  Command::new("daemon")
    .about("run daemon")
    .arg(arg_eth_address())
    .arg(arg_rpc_address())
    .arg(arg_eth_master())
}

fn arg_eth_address() -> Arg<'static> {
  Arg::new(ARG_ETH_ADDRESS)
    .long(ARG_ETH_ADDRESS)
    .takes_value(true)
    .value_name("ADDRESS")
    .required(true)
    .help("ethereum JSON-RPC address")
}

fn arg_eth_master() -> Arg<'static> {
  Arg::new(ARG_ETH_MASTER)
    .long(ARG_ETH_MASTER)
    .takes_value(true)
    .value_name("ADDRESS")
    .validator(web3::types::Address::from_str)
    .required(false)
    .help("ethereum address of the master record contract")
}

pub fn run(matches: &ArgMatches) -> Result<(), Box<dyn std::error::Error>> {
  let daemon_opts = DaemonOpts {
    eth_addr: matches.value_of_t(ARG_ETH_ADDRESS)?,
    rpc_addr: matches.value_of_t(ARG_RPC_ADDRESS)?,
    master_addr: matches
      .value_of(ARG_ETH_MASTER)
      .map(web3::types::Address::from_str)
      .transpose()?,
  };
  tokio::runtime::Builder::new_multi_thread()
    .enable_all()
    .build()
    .unwrap()
    .block_on(p2pim::daemon::listen_and_serve(daemon_opts))
}
