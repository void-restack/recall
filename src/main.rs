mod cli;
mod commands;
mod memory;
mod paths;
mod search;
mod secrets;
mod shell;
mod store;
mod tui;

use clap::Parser;

use crate::cli::{Cli, Command};

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Add(args)) => commands::add(args),
        Some(Command::List) => commands::list(),
        Some(Command::Search(args)) => commands::search(args),
        Some(Command::Get(args)) => commands::get(args),
        Some(Command::Edit(args)) => commands::edit(args),
        Some(Command::Delete(args)) => commands::delete(args),
        Some(Command::Export) => commands::export(),
        Some(Command::Import(args)) => commands::import(args),
        Some(Command::Init(args)) => commands::init(args),
        None => commands::pick(),
    }
}
