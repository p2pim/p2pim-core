use crate::cmd::{arg_token, arg_url, ARG_TOKEN, ARG_URL};
use clap::{ArgMatches, Command};
use ethcontract::U256;
use p2pim::proto::api::p2pim_client::P2pimClient;
use p2pim::proto::api::ApproveRequest;
use web3::types::H256;

pub fn command<'a>() -> Command<'a> {
  Command::new("approve")
    .about("approve to use tokens by the adjudicator")
    .arg(arg_url())
    .arg(arg_token())
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
  let response = client.approve(req).await?;
  let trans_hash: H256 = response
    .get_ref()
    .transaction_hash
    .as_ref()
    .ok_or("unexpected empty transaction hash response")?
    .into();
  println!("Approval sent, transaction 0x{:x}", trans_hash);
  Ok(())
}
