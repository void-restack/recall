mod cli;
mod commands;
mod history;
mod line_editor;
mod memory;
mod paths;
mod search;
mod secrets;
mod shell;
mod store;
mod theme;
mod tui;

use clap::Parser;

use crate::cli::{Cli, Command};

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    paths::set_db_override(cli.db);
    match cli.command {
        Some(Command::Add(args)) => commands::add(args),
        Some(Command::List(args)) => commands::list(args),
        Some(Command::Search(args)) => commands::search(args),
        Some(Command::Get(args)) => commands::get(args),
        Some(Command::Edit(args)) => commands::edit(args),
        Some(Command::Delete(args)) => commands::delete(args),
        Some(Command::Export) => commands::export(),
        Some(Command::Import(args)) => commands::import(args),
        Some(Command::History(args)) => commands::history(args),
        Some(Command::Init(args)) => commands::init(args),
        None => commands::pick(),
    }
}
