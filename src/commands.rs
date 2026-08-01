use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;

use crate::cli::AddArgs;
use crate::memory::{self, NewMemory};
use crate::store::Store;

pub fn add(args: AddArgs) -> Result<()> {
    let store = Store::open()?;
    let new = NewMemory {
        command: args.command,
        description: clean_description(args.description),
        tags: memory::normalize_tags(args.tag),
    };
    let saved = store.insert(&new, now_millis())?;

    // id to stdout (scriptable), status to stderr.
    println!("{}", saved.id);
    let kind = if saved.is_draft() { "draft " } else { "" };
    eprintln!("saved {kind}#{}", saved.id);
    Ok(())
}

pub fn list() -> Result<()> {
    let store = Store::open()?;
    let memories = store.list()?;
    if memories.is_empty() {
        eprintln!("no memories yet — capture one with `recall add`");
        return Ok(());
    }
    for m in &memories {
        let label = m
            .description
            .as_deref()
            .filter(|d| !d.is_empty())
            .unwrap_or(&m.command);
        let draft = if m.is_draft() { " *" } else { "" };
        let tags = if m.tags.is_empty() {
            String::new()
        } else {
            format!("  [{}]", m.tags.join(", "))
        };
        println!("{:>4}{draft}  {label}{tags}", m.id);
    }
    Ok(())
}

fn clean_description(raw: Option<String>) -> Option<String> {
    // Whitespace-only counts as absent, keeping the Memory a Draft.
    raw.map(|d| d.trim().to_string()).filter(|d| !d.is_empty())
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
