use clap::Arg;
use std::str::FromStr;

pub mod approve;
pub mod daemon;
pub mod deposit;
pub mod info;
pub mod swarm;

const ARG_URL: &str = "url";
const ARG_URL_DEFAULT: &str = "http://127.0.0.1:8122";

fn arg_url() -> Arg<'static> {
  Arg::new(ARG_URL)
    .long(ARG_URL)
    .takes_value(true)
    .value_name("URL")
    .default_value(ARG_URL_DEFAULT)
    .help("specify the url of the daemon")
}

const ARG_TOKEN: &str = "token";

fn arg_token() -> Arg<'static> {
  Arg::new(ARG_TOKEN)
    .takes_value(true)
    .required(true)
    .validator(web3::types::Address::from_str)
    .help("token to approve")
}
