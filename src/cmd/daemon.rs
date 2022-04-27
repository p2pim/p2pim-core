use clap::{Arg, ArgMatches, Command};

const ARG_ETH_ADDRESS: &str = "eth.address";

const ARG_RPC_ADDRESS: &str = "rpc.address";
const ARG_RPC_ADDRESS_DEFAULT: &str = "127.0.0.1:8122";

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
}

fn arg_eth_address() -> Arg<'static> {
  Arg::new(ARG_ETH_ADDRESS)
    .long(ARG_ETH_ADDRESS)
    .takes_value(true)
    .value_name("ADDRESS")
    .required(true)
    .help("ethereum JSON-RPC address")
}

pub fn run(matches: &ArgMatches) -> Result<(), Box<dyn std::error::Error>> {
  let eth_addr = matches.value_of_t(ARG_ETH_ADDRESS)?;
  let rpc_addr = matches.value_of_t(ARG_RPC_ADDRESS)?;
  tokio::runtime::Builder::new_multi_thread()
    .enable_all()
    .build()
    .unwrap()
    .block_on(p2pim::daemon::listen_and_serve(eth_addr, rpc_addr))
}
