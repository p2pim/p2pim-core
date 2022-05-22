use crate::cmd::{arg_url, ARG_URL};
use clap::{ArgMatches, Command};
use libp2p::PeerId;
use p2pim::proto::api::swarm_client::SwarmClient;
use p2pim::proto::api::GetConnectedPeersRequest;

const CMD_PEERS: &str = "peers";

pub fn command<'a>() -> Command<'a> {
  Command::new("swarm")
    .about("swarm related commands")
    .subcommand_required(true)
    .arg_required_else_help(true)
    .subcommand(command_peers())
}

fn command_peers<'a>() -> Command<'a> {
  Command::new(CMD_PEERS).about("lists connected peers").arg(arg_url())
}

pub fn run(matches: &ArgMatches) -> Result<(), Box<dyn std::error::Error>> {
  match matches.subcommand() {
    Some((CMD_PEERS, m)) => run_peers(m),
    _ => unreachable!("this should not happen if we have all the cases covered"),
  }
}

pub fn run_peers(matches: &ArgMatches) -> Result<(), Box<dyn std::error::Error>> {
  let rpc_url = matches.value_of_t(ARG_URL)?;
  tokio::runtime::Builder::new_multi_thread()
    .enable_all()
    .build()
    .unwrap()
    .block_on(run_peers_async(rpc_url))
}

async fn run_peers_async(rpc_url: String) -> Result<(), Box<dyn std::error::Error>> {
  let mut client = SwarmClient::connect(rpc_url).await?;
  let req = GetConnectedPeersRequest {};
  let response = client.get_connected_peers(req).await?;
  let result = response
    .get_ref()
    .peer_list
    .iter()
    .enumerate()
    .map(|(i, p)| PeerId::from_bytes(p.data.as_slice()).map(|c| format!("{}: {}", i, c)))
    .collect::<Result<Vec<String>, _>>()?
    .join("\n");
  if result.is_empty() {
    println!("no peers")
  } else {
    println!("{}", result);
  }
  Ok(())
}
