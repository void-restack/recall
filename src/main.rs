mod cli;
mod commands;
mod memory;
mod paths;
mod store;

use clap::Parser;

use crate::cli::{Cli, Command};

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Add(args) => commands::add(args),
        Command::List => commands::list(),
    }
}
