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

    /// Search Memories by fuzzy, typo-tolerant matching
    Search(SearchArgs),

    /// Print a Memory's command to stdout (and count it as used)
    Get(GetArgs),

    /// Export all Memories as JSONL
    Export,
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

#[derive(Args)]
pub struct SearchArgs {
    /// Words to match across command, description, and tags
    #[arg(required = true)]
    pub query: Vec<String>,

    /// Maximum number of results to show
    #[arg(short, long, default_value_t = 20)]
    pub limit: usize,
}

#[derive(Args)]
pub struct GetArgs {
    /// The id shown by `list` or `search`
    pub id: i64,
}
