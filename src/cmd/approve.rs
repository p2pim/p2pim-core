use crate::cmd::{arg_url, ARG_URL};
use clap::{Arg, ArgMatches, Command};
use ethcontract::U256;
use p2pim::proto::api::p2pim_client::P2pimClient;
use p2pim::proto::api::ApproveRequest;
use std::str::FromStr;
use web3::types::H256;

const ARG_TOKEN: &str = "token";

pub fn command() -> Command<'static> {
  Command::new("approve")
    .about("approve to use tokens by the adjudicator")
    .arg(arg_url())
    .arg(arg_token())
}

fn arg_token() -> Arg<'static> {
  Arg::new(ARG_TOKEN)
    .takes_value(true)
    .required(true)
    .validator(web3::types::Address::from_str)
    .help("token to approve")
}

pub fn run(matches: &ArgMatches) -> Result<(), Box<dyn std::error::Error>> {
  let rpc_url = matches.value_of_t(ARG_URL)?;
  let token_addr = matches.value_of_t(ARG_TOKEN)?;
  tokio::runtime::Builder::new_multi_thread()
    .enable_all()
    .build()
    .unwrap()
    .block_on(run_approve(rpc_url, token_addr))
}

async fn run_approve(rpc_url: String, token_addr: web3::types::Address) -> Result<(), Box<dyn std::error::Error>> {
  let mut client = P2pimClient::connect(rpc_url).await?;
  let req = ApproveRequest {
    token_address: Some(From::from(token_addr)),
    amount: Some(From::from(U256::max_value())),
  };
  let result = client.approve(req).await?;
  let trans_hash: H256 = result
    .get_ref()
    .transaction_hash
    .as_ref()
    .ok_or("unexpected empty transaction hash response")?
    .into();
  println!("Approval sent, transaction 0x{:x}", trans_hash);
  Ok(())
}
