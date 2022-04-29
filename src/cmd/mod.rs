use clap::Arg;

pub mod approve;
pub mod daemon;
pub mod info;

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
