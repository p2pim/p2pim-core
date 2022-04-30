use clap::Command;
use std::error::Error;

pub mod cmd;

fn main() -> Result<(), Box<dyn Error>> {
  env_logger::init();
  let matches = cli().get_matches();
  let result = match matches.subcommand() {
    Some(("approve", m)) => cmd::approve::run(m),
    Some(("daemon", m)) => cmd::daemon::run(m),
    Some(("deposit", m)) => cmd::deposit::run(m),
    Some(("info", m)) => cmd::info::run(m),
    _ => unreachable!("this should not happen if we have all the cases covered"),
  };
  result
}

fn cli() -> Command<'static> {
  Command::new(env!("CARGO_BIN_NAME"))
    .about("P2pim decentralized storage")
    .subcommand_required(true)
    .arg_required_else_help(true)
    .subcommand(cmd::approve::command())
    .subcommand(cmd::daemon::command())
    .subcommand(cmd::deposit::command())
    .subcommand(cmd::info::command())
}
