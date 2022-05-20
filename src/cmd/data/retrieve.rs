use crate::cmd::{arg_url, ARG_URL};
use clap::{Arg, ArgMatches, Command};
use libp2p::PeerId;
use p2pim::proto::api::p2pim_client::P2pimClient;
use p2pim::proto::api::RetrieveRequest;
use tokio::io::AsyncWriteExt;

pub const CMD_NAME: &str = "retrieve";

const ARG_PEER_ID: &str = "peer";
const ARG_NONCE: &str = "nonce";

pub fn command() -> Command<'static> {
  Command::new(CMD_NAME)
    .about("retrieve data from peer")
    .arg(arg_url())
    .arg(arg_peer_id())
    .arg(arg_nonce())
}

fn arg_nonce() -> Arg<'static> {
  Arg::new(ARG_NONCE)
    .takes_value(true)
    .required(true)
    .validator(str::parse::<u64>)
    .help("nonce to challenge")
}

fn arg_peer_id() -> Arg<'static> {
  Arg::new(ARG_PEER_ID)
    .takes_value(true)
    .required(true)
    .help("peer of the lease")
}

pub fn run(matches: &ArgMatches) -> Result<(), Box<dyn std::error::Error>> {
  let rpc_url = matches.value_of_t(ARG_URL)?;
  let peer_id = matches.value_of_t(ARG_PEER_ID)?;
  let nonce = matches.value_of_t(ARG_NONCE)?;
  tokio::runtime::Builder::new_multi_thread()
    .enable_all()
    .build()
    .unwrap()
    .block_on(run_retrieve(rpc_url, peer_id, nonce))
}

async fn run_retrieve(rpc_url: String, peer_id: PeerId, nonce: u64) -> Result<(), Box<dyn std::error::Error>> {
  let mut client = P2pimClient::connect(rpc_url).await?;
  let retrieve_request = RetrieveRequest {
    peer_id: Some(peer_id.into()),
    nonce,
  };
  let response = client.retrieve(retrieve_request).await?;
  let data = response.into_inner().data;
  let mut stdout = tokio::io::stdout();
  stdout.write_all(data.as_slice()).await?;
  Ok(())
}
