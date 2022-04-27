use clap::{ArgMatches, Command};
use std::error::Error;

use crate::cmd::{arg_url, ARG_URL};
use p2pim::proto::api::p2pim_client::P2pimClient;
use p2pim::proto::api::{BalanceEntry, GetInfoRequest};

pub fn command() -> Command<'static> {
  Command::new("info").about("show p2pim account info").arg(arg_url())
}

pub fn run(matches: &ArgMatches) -> Result<(), Box<dyn std::error::Error>> {
  let rpc_url = matches.value_of_t(ARG_URL)?;
  tokio::runtime::Builder::new_multi_thread()
    .enable_all()
    .build()
    .unwrap()
    .block_on(run_info(rpc_url))
}

async fn run_info(rpc_url: String) -> Result<(), Box<dyn std::error::Error>> {
  let mut client = P2pimClient::connect(rpc_url).await?;
  let get_info_req: GetInfoRequest = Default::default();
  let response = client.get_info(get_info_req).await?;
  let response_dto = response.get_ref();
  let address: web3::types::Address = convert_or_err(response_dto.address.as_ref(), "empty address")?;
  let balance = response_dto
    .balance
    .iter()
    .map(format_balance)
    .collect::<Result<Vec<String>, _>>()
    .map(|bal| bal.join("\n"))?;
  println!("Address: {}", address);
  println!("Balances:");
  println!("{}", balance);
  Ok(())
}

fn format_balance(entry: &BalanceEntry) -> Result<String, Box<dyn Error>> {
  let token_address: web3::types::Address = convert_or_err(entry.token.as_ref(), "missing token address")?;
  let amount: web3::types::U256 = convert_or_err(entry.available.as_ref(), "missing available amount")?;
  Ok(format!("  Token: {}, Amount: {}", token_address, amount))
}

fn convert_or_err<I, O: From<I>, E>(input: Option<I>, err: E) -> Result<O, E> {
  input.map(Into::<O>::into).ok_or(err)
}
