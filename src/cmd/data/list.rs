use crate::cmd::{arg_url, ARG_URL};
use chrono::{DateTime, NaiveDateTime, Utc};
use clap::{ArgMatches, Command};
use p2pim::proto::api::p2pim_client::P2pimClient;
use p2pim::proto::api::ListStorageRentedRequest;
use std::convert::TryFrom;

pub const LIST_CMD: &str = "list";

pub fn command() -> Command<'static> {
  Command::new(LIST_CMD).about("list rented storage").arg(arg_url())
}

pub fn run(matches: &ArgMatches) -> Result<(), Box<dyn std::error::Error>> {
  let rpc_url = matches.value_of_t(ARG_URL)?;
  tokio::runtime::Builder::new_multi_thread()
    .enable_all()
    .build()
    .unwrap()
    .block_on(run_list(rpc_url))
}

async fn run_list(rpc_url: String) -> Result<(), Box<dyn std::error::Error>> {
  let mut client = P2pimClient::connect(rpc_url).await?;
  let list_storage_request = ListStorageRentedRequest {};
  let response = client.list_storage_rented(list_storage_request).await?;
  for (i, data) in response.get_ref().storage_rented_data.iter().enumerate() {
    let peer_id = data.peer_id.as_ref().map(libp2p::PeerId::try_from).ok_or("empty peer_id")??;
    let nonce = data.nonce;
    println!("{}: {} - {}", i, peer_id, nonce);

    let duration = data
      .lease_duration
      .clone()
      .map(std::time::Duration::try_from)
      .ok_or("empty lease_duration")?
      .map_err(|_| "negative lease_duration")?;

    let tx_hash = data.transaction_hash.as_ref().map(web3::types::H256::from);
    let tx_ts = data.lease_started.clone();
    println!("  Lease Duration  : {:?}", duration);
    if let (Some(hash), Some(ts)) = (tx_hash, tx_ts) {
      let ts2 = DateTime::<Utc>::from_utc(NaiveDateTime::from_timestamp(ts.seconds, 0), Utc);
      println!("  Transaction Hash : 0x{:x}", hash);
      println!("  Transaction Start: {}", ts2);
      println!(
        "  Lease Ends       : {}",
        ts2 + chrono::Duration::seconds(duration.as_secs() as i64)
      );
    } else {
      println!("  Transaction Hash: Not confirmed",);
    }
  }

  Ok(())
}
