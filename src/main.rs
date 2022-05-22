use clap::Command;
use std::error::Error;
use typed_arena::Arena;

pub mod cmd;

fn main() -> Result<(), Box<dyn Error>> {
  env_logger::init();

  let mut buf = Arena::new();

  let matches = cli(&mut buf).get_matches();
  let result = match matches.subcommand() {
    Some(("approve", m)) => cmd::approve::run(m),
    Some((cmd::daemon::CMD_NAME, m)) => cmd::daemon::run(m),
    Some(("deposit", m)) => cmd::deposit::run(m),
    Some(("info", m)) => cmd::info::run(m),
    Some(("swarm", m)) => cmd::swarm::run(m),
    Some((cmd::withdraw::CMD_NAME, m)) => cmd::withdraw::run(m),
    Some((cmd::data::DATA_CMD, m)) => cmd::data::run(m),
    _ => unreachable!("this should not happen if we have all the cases covered"),
  };
  result
}

fn cli(buf: &mut Arena<String>) -> Command {
  Command::new(env!("CARGO_BIN_NAME"))
    .about("P2pim decentralized storage")
    .subcommand_required(true)
    .arg_required_else_help(true)
    .subcommand(cmd::approve::command())
    .subcommand(cmd::daemon::command(buf))
    .subcommand(cmd::deposit::command())
    .subcommand(cmd::info::command())
    .subcommand(cmd::data::command())
    .subcommand(cmd::swarm::command())
    .subcommand(cmd::withdraw::command())
}
