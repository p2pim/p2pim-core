use clap::{ArgMatches, Command};

pub mod list;
pub mod store;

pub const DATA_CMD: &str = "data";

pub fn command() -> Command<'static> {
  Command::new(DATA_CMD)
    .about("data related commands")
    .subcommand_required(true)
    .arg_required_else_help(true)
    .subcommand(list::command())
    .subcommand(store::command())
}

pub fn run(matches: &ArgMatches) -> Result<(), Box<dyn std::error::Error>> {
  match matches.subcommand() {
    Some((list::LIST_CMD, m)) => list::run(m),
    Some((store::STORE_CMD, m)) => store::run(m),
    _ => unreachable!("this should not happen if we have all the cases covered"),
  }
}
