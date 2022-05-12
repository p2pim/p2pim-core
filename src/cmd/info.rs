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
  let address_wallet: web3::types::Address = convert_or_err(response_dto.address_wallet.as_ref(), "empty address wallet")?;
  let address_storage: web3::types::Address = convert_or_err(response_dto.address_storage.as_ref(), "empty address storage")?;
  let balance = response_dto
    .balance
    .iter()
    .map(format_balance)
    .collect::<Result<Vec<String>, _>>()
    .map(|bal| bal.join("\n"))?;
  println!("Wallet  Address: 0x{:x}", address_wallet);
  println!("Storage Address: 0x{:x}", address_storage);
  println!("Balances:");
  println!("{}", balance);
  Ok(())
}

fn format_balance(entry: &BalanceEntry) -> Result<String, Box<dyn Error>> {
  let token = entry.token_metadata.as_ref().ok_or("missing token info")?;

  let token_address: web3::types::Address = convert_or_err(entry.token_address.as_ref(), "missing token address")?;
  let token_name = &token.name;
  let token_symbol = &token.symbol;

  let mut result = {
    if token_name.is_empty() {
      format!("  Token at 0x{:x} :\n", token_address)
    } else {
      let symbol = if token_symbol.is_empty() {
        Default::default()
      } else {
        format!(" ({})", token_symbol)
      };
      format!("  {}{} at 0x{:x} :\n", token_name, symbol, token_address)
    }
  };

  let token_decimals = From::from(token.decimals);

  let to_big_decimal = |v| BigDecimal::new(v, token_decimals);
  let available_account = convert_or_err(
    entry.wallet_balance.as_ref().and_then(|e| e.available.as_ref()),
    "missing available account amount",
  )
  .map(to_big_decimal)?;
  let allowed_account = convert_or_err(
    entry.wallet_balance.as_ref().and_then(|e| e.allowance.as_ref()),
    "missing allowed account amount",
  )
  .map(to_big_decimal)?;
  let available_p2pim = convert_or_err(
    entry.storage_balance.as_ref().and_then(|s| s.available.as_ref()),
    "missing available p2p amount",
  )
  .map(to_big_decimal)?;
  let locked_rents = convert_or_err(
    entry.storage_balance.as_ref().and_then(|s| s.locked_rents.as_ref()),
    "missing locked rents amount",
  )
  .map(to_big_decimal)?;
  let locked_lets = convert_or_err(
    entry.storage_balance.as_ref().and_then(|s| s.locked_lets.as_ref()),
    "missing locked lets amount",
  )
  .map(to_big_decimal)?;

  writeln!(result, "    Available Account: {}", available_account)?;
  writeln!(result, "    Allowed Account  : {}", allowed_account)?;
  writeln!(result, "    Available P2pim  : {}", available_p2pim)?;
  writeln!(result, "    Locked Rents     : {}", locked_rents)?;
  writeln!(result, "    Locked Lets      : {}", locked_lets)?;
  Ok(result)
}

fn convert_or_err<I, O: From<I>, E>(input: Option<I>, err: E) -> Result<O, E> {
  input.map(Into::<O>::into).ok_or(err)
}
