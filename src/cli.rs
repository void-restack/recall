use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(name = "recall", version, about = "Local-first command memory for your terminal")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Save a command as a Memory
    Add(AddArgs),
    /// List saved Memories, newest first
    List,
}

#[derive(Args)]
pub struct AddArgs {
    /// The command to remember
    pub command: String,

    /// Short description of what it is for. Omit to save a Draft and annotate later.
    #[arg(short, long)]
    pub description: Option<String>,

    /// Tag to attach; repeat for several (e.g. -t docker -t cleanup)
    #[arg(short, long = "tag")]
    pub tag: Vec<String>,
}
