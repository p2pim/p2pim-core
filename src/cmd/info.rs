use bigdecimal::BigDecimal;
use std::error::Error;
use std::fmt::Write;

use crate::cmd::{arg_url, ARG_URL};
use clap::{ArgMatches, Command};
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
  println!("Address: 0x{:x}", address);
  println!("Balances:");
  println!("{}", balance);
  Ok(())
}

fn format_balance(entry: &BalanceEntry) -> Result<String, Box<dyn Error>> {
  let token = entry.token.as_ref().ok_or("missing token info")?;

  let token_address: web3::types::Address = convert_or_err(token.token_address.as_ref(), "missing token address")?;
  let token_name = &token.name;
  let token_symbol = &token.symbol;

  let mut result = {
    if token_name.is_empty() {
      format!("  Token at 0x{:x}:\n", token_address)
    } else {
      let symbol = if token_symbol.is_empty() {
        Default::default()
      } else {
        format!(" ({})", token_symbol)
      };
      format!("  {}{} at 0x{:x}:\n", token_name, symbol, token_address)
    }
  };

  let token_decimals = From::from(token.decimals);

  let to_big_decimal = |v| BigDecimal::new(v, token_decimals);
  let available = convert_or_err(entry.available.as_ref(), "missing available amount").map(to_big_decimal)?;
  let allowed = convert_or_err(entry.allowed.as_ref(), "missing available amount").map(to_big_decimal)?;
  let supplied = convert_or_err(entry.supplied.as_ref(), "missing available amount").map(to_big_decimal)?;

  write!(result, "    Available: {}\n", available)?;
  write!(result, "    Allowed  : {}\n", allowed)?;
  write!(result, "    Supplied : {}\n", supplied)?;
  Ok(result)
}

fn convert_or_err<I, O: From<I>, E>(input: Option<I>, err: E) -> Result<O, E> {
  input.map(Into::<O>::into).ok_or(err)
}
