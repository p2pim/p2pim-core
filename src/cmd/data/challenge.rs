use crate::cmd::{arg_url, ARG_URL};
use clap::{Arg, ArgMatches, Command};
use libp2p::PeerId;
use p2pim::proto::api::p2pim_client::P2pimClient;
use p2pim::proto::api::ChallengeRequest;

pub const CMD_NAME: &str = "challenge";

const ARG_PEER_ID: &str = "peer";
const ARG_NONCE: &str = "nonce";
const ARG_BLOCK_NUMBER: &str = "block.number";

pub fn command<'a>() -> Command<'a> {
  Command::new(CMD_NAME)
    .about("challenge lease to peer")
    .arg(arg_url())
    .arg(arg_peer_id())
    .arg(arg_nonce())
    .arg(arg_block())
}

fn arg_nonce<'a>() -> Arg<'a> {
  Arg::new(ARG_NONCE)
    .takes_value(true)
    .required(true)
    .validator(str::parse::<u64>)
    .help("nonce to challenge")
}

fn arg_peer_id<'a>() -> Arg<'a> {
  Arg::new(ARG_PEER_ID)
    .takes_value(true)
    .required(true)
    .help("peer of the lease")
}

fn arg_block<'a>() -> Arg<'a> {
  Arg::new(ARG_BLOCK_NUMBER)
    .takes_value(true)
    .required(true)
    .validator(str::parse::<u32>)
    .help("block to request")
}

pub fn run(matches: &ArgMatches) -> Result<(), Box<dyn std::error::Error>> {
  let rpc_url = matches.value_of_t(ARG_URL)?;
  let peer_id = matches.value_of_t(ARG_PEER_ID)?;
  let nonce = matches.value_of_t(ARG_NONCE)?;
  let block_number = matches.value_of_t(ARG_BLOCK_NUMBER)?;
  tokio::runtime::Builder::new_multi_thread()
    .enable_all()
    .build()
    .unwrap()
    .block_on(run_challenge(rpc_url, peer_id, nonce, block_number))
}

async fn run_challenge(
  rpc_url: String,
  peer_id: PeerId,
  nonce: u64,
  block_number: u32,
) -> Result<(), Box<dyn std::error::Error>> {
  let mut client = P2pimClient::connect(rpc_url).await?;
  let challenge_request = ChallengeRequest {
    peer_id: Some(peer_id.into()),
    nonce,
    block_number,
  };
  let _ = client.challenge(challenge_request).await?;
  println!("Challenge Ok");
  Ok(())
}
