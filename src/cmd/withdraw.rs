use crate::cmd::{arg_amount, arg_token, arg_url, ARG_AMOUNT, ARG_TOKEN, ARG_URL};
use bigdecimal::BigDecimal;
use clap::{ArgMatches, Command};
use num_bigint::{Sign, ToBigInt};
use p2pim::proto::api::p2pim_client::P2pimClient;
use p2pim::proto::api::{GetBalanceRequest, WithdrawRequest};
use std::convert::TryInto;
use web3::types::H256;

pub const CMD_NAME: &str = "withdraw";

pub fn command() -> Command<'static> {
  Command::new(CMD_NAME)
    .about("withdraw tokens from adjudicator")
    .arg(arg_url())
    .arg(arg_token())
    .arg(arg_amount())
}

pub fn run(matches: &ArgMatches) -> Result<(), Box<dyn std::error::Error>> {
  let rpc_url = matches.value_of_t(ARG_URL)?;
  let token_addr = matches.value_of_t(ARG_TOKEN)?;
  let amount = matches.value_of_t(ARG_AMOUNT)?;
  tokio::runtime::Builder::new_multi_thread()
    .enable_all()
    .build()
    .unwrap()
    .block_on(run_withdraw(rpc_url, token_addr, amount))
}

async fn run_withdraw(
  rpc_url: String,
  token_addr: web3::types::Address,
  amount: BigDecimal,
) -> Result<(), Box<dyn std::error::Error>> {
  let mut client = P2pimClient::connect(rpc_url).await?;
  let get_balance_request = GetBalanceRequest {
    token_address: Some(token_addr.into()),
  };
  let response = client.get_balance(get_balance_request).await?;
  let decimals = response
    .get_ref()
    .balance
    .as_ref()
    .and_then(|v| v.token.as_ref())
    .map(|v| v.decimals)
    .ok_or("TODO: invalid response")? as i64;
  let abs_amount: BigDecimal = amount * BigDecimal::new(1.into(), -decimals);
  if !abs_amount.is_integer() {
    return Err("TODO(formatting): the amount has too many decimals".into());
  } else if abs_amount.sign() == Sign::Minus {
    return Err("TODO:(formatting): the amount cannot be negative".into());
  } else {
    let conv_amount = abs_amount.to_bigint().expect("never returns None").try_into()?;
    let response = client
      .withdraw(WithdrawRequest {
        token_address: Some(token_addr.into()),
        amount: Some(conv_amount),
      })
      .await?;
    let trans_hash: H256 = response
      .get_ref()
      .transaction_hash
      .as_ref()
      .ok_or("unexpected empty transaction hash response")?
      .into();
    println!("Withdraw sent, transaction 0x{:x}", trans_hash);
    Ok(())
  }
}
