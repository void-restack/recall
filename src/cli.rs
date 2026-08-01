use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "recall", version, about = "Local-first command memory for your terminal")]
pub struct Cli {
    /// Use a specific database file instead of the default (also via RECALL_DB)
    #[arg(long, global = true, env = "RECALL_DB", value_name = "PATH")]
    pub db: Option<std::path::PathBuf>,

    /// With no subcommand, `recall` opens the interactive picker.
    #[command(subcommand)]
    pub command: Option<Command>,
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

    /// Edit a Memory's command, description, or tags
    Edit(EditArgs),

    /// Delete a Memory (asks for confirmation)
    Delete(DeleteArgs),

    /// Export all Memories as JSONL
    Export,

    /// Import Memories from a JSONL file (adds to the collection)
    Import(ImportArgs),

    /// Print shell integration that enables `recall add --last`
    Init(InitArgs),
}

#[derive(Args)]
pub struct AddArgs {
    /// The command to remember (omit when using --last)
    pub command: Option<String>,

    /// Capture the previous command from your shell (requires `recall init`)
    #[arg(long)]
    pub last: bool,

    /// Save even if the command looks like it contains a secret
    #[arg(short, long)]
    pub force: bool,

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

#[derive(Args)]
pub struct EditArgs {
    /// The id to edit
    pub id: i64,

    /// Replace the command text
    #[arg(short = 'c', long)]
    pub command: Option<String>,

    /// Set the description; pass an empty string to clear it (back to a Draft)
    #[arg(short, long)]
    pub description: Option<String>,

    /// Replace the whole tag set; repeat for several
    #[arg(short, long = "tag")]
    pub tag: Vec<String>,

    /// Remove all tags
    #[arg(long)]
    pub clear_tags: bool,
}

#[derive(Args)]
pub struct DeleteArgs {
    /// The id to delete
    pub id: i64,

    /// Skip the confirmation prompt
    #[arg(short = 'y', long)]
    pub yes: bool,
}

#[derive(Args)]
pub struct ImportArgs {
    /// Path to a JSONL file (as produced by `recall export`)
    pub file: std::path::PathBuf,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
}

#[derive(Args)]
pub struct InitArgs {
    /// Which shell to generate integration for
    #[arg(value_enum)]
    pub shell: Shell,
}
