mod cli;
mod commands;
mod memory;
mod paths;
mod search;
mod shell;
mod store;

use clap::Parser;

use crate::cli::{Cli, Command};

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Add(args) => commands::add(args),
        Command::List => commands::list(),
        Command::Search(args) => commands::search(args),
        Command::Get(args) => commands::get(args),
        Command::Edit(args) => commands::edit(args),
        Command::Delete(args) => commands::delete(args),
        Command::Export => commands::export(),
        Command::Init(args) => commands::init(args),
    }
}
