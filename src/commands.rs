use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};

use crate::cli::{AddArgs, DeleteArgs, EditArgs, GetArgs, ImportArgs, InitArgs, SearchArgs, Shell};
use crate::memory::{self, CommandMemory, ImportRecord, NewMemory};
use crate::store::Store;
use crate::{paths, shell};

pub fn add(args: AddArgs) -> Result<()> {
    let command = resolve_command(args.command, args.last)?;
    warn_about_secrets(&command, args.force)?;
    let store = Store::open()?;
    let duplicates = store.ids_with_command(&command)?;
    let new = NewMemory {
        command,
        description: clean_description(args.description),
        tags: memory::normalize_tags(args.tag),
    };
    let saved = store.insert(&new, now_millis())?;

    // id to stdout (scriptable), status to stderr.
    println!("{}", saved.id);
    let kind = if saved.is_draft() { "draft " } else { "" };
    eprintln!("saved {kind}#{}", saved.id);
    if !duplicates.is_empty() {
        let ids: Vec<String> = duplicates.iter().map(|id| format!("#{id}")).collect();
        eprintln!("note: same command already saved as {}", ids.join(", "));
    }
    Ok(())
}

fn resolve_command(command: Option<String>, last: bool) -> Result<String> {
    match (command, last) {
        (Some(_), true) => anyhow::bail!("pass a command or --last, not both"),
        (Some(c), false) => Ok(c),
        (None, true) => read_last_command(),
        (None, false) => anyhow::bail!("provide a command, or --last to capture the previous one"),
    }
}

fn read_last_command() -> Result<String> {
    let path = paths::last_command_path()?;
    let captured = std::fs::read_to_string(&path).unwrap_or_default();
    let command = captured.trim();
    if command.is_empty() {
        anyhow::bail!(
            "no recent command captured — install the shell hook first:\n  eval \"$(recall init zsh)\"   # or bash"
        );
    }
    Ok(command.to_string())
}

pub fn init(args: InitArgs) -> Result<()> {
    let last_file = paths::last_command_path()?;
    let script = match args.shell {
        Shell::Zsh => shell::zsh(&last_file),
        Shell::Bash => shell::bash(&last_file),
    };
    print!("{script}");
    Ok(())
}

pub fn pick() -> Result<()> {
    use std::io::IsTerminal;
    if !std::io::stderr().is_terminal() {
        anyhow::bail!("not a terminal — use `recall search <words>` instead");
    }
    let store = Store::open()?;
    let memories = store.list()?;
    if memories.is_empty() {
        eprintln!("no memories yet — capture one with `recall add`");
        return Ok(());
    }
    if let Some(index) = crate::tui::run(&memories)? {
        let chosen = &memories[index];
        store.record_use(chosen.id, now_millis())?;
        println!("{}", chosen.command);
    }
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
        print_row(m);
    }
    Ok(())
}

pub fn search(args: SearchArgs) -> Result<()> {
    let query = args.query.join(" ");
    let store = Store::open()?;
    let memories = store.list()?;
    let hits = crate::search::search(&query, &memories, args.limit);
    if hits.is_empty() {
        eprintln!("no matches for `{query}`");
        return Ok(());
    }
    for m in hits {
        print_row(m);
    }
    Ok(())
}

pub fn get(args: GetArgs) -> Result<()> {
    let store = Store::open()?;
    match store.get(args.id)? {
        Some(m) => {
            println!("{}", m.command);
            store.record_use(m.id, now_millis())?;
            Ok(())
        }
        None => anyhow::bail!("no memory #{}", args.id),
    }
}

pub fn export() -> Result<()> {
    let store = Store::open()?;
    for m in store.list()? {
        println!("{}", serde_json::to_string(&m)?);
    }
    Ok(())
}

pub fn import(args: ImportArgs) -> Result<()> {
    use std::io::BufRead;
    let file = std::fs::File::open(&args.file)
        .with_context(|| format!("opening {}", args.file.display()))?;

    // Validate every line before touching the store (see brief §15).
    let mut records = Vec::new();
    for (n, line) in std::io::BufReader::new(file).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let record: ImportRecord =
            serde_json::from_str(&line).with_context(|| format!("line {}: invalid record", n + 1))?;
        if record.command.trim().is_empty() {
            anyhow::bail!("line {}: record has an empty command", n + 1);
        }
        records.push(record);
    }

    let count = Store::open()?.import_all(&records, now_millis())?;
    eprintln!("imported {count} memories");
    Ok(())
}

pub fn edit(args: EditArgs) -> Result<()> {
    let store = Store::open()?;
    let mut m = store
        .get(args.id)?
        .ok_or_else(|| anyhow!("no memory #{}", args.id))?;

    let mut changed = false;
    if let Some(command) = args.command {
        m.command = command;
        changed = true;
    }
    if let Some(description) = args.description {
        m.description = clean_description(Some(description));
        changed = true;
    }
    if args.clear_tags {
        m.tags.clear();
        changed = true;
    } else if !args.tag.is_empty() {
        m.tags = memory::normalize_tags(args.tag);
        changed = true;
    }
    if !changed {
        anyhow::bail!("nothing to change — pass -c, -d, or -t");
    }

    store.update(&m, now_millis())?;
    eprintln!("updated #{}", m.id);
    Ok(())
}

pub fn delete(args: DeleteArgs) -> Result<()> {
    let store = Store::open()?;
    let m = store
        .get(args.id)?
        .ok_or_else(|| anyhow!("no memory #{}", args.id))?;

    if !args.yes && !confirm(&m)? {
        eprintln!("cancelled");
        return Ok(());
    }
    store.delete(m.id)?;
    eprintln!("deleted #{}", m.id);
    Ok(())
}

fn confirm(m: &CommandMemory) -> Result<bool> {
    prompt_yes(&format!("delete #{}  {} ? [y/N] ", m.id, m.command))
}

/// Warn if the command looks like it holds a secret. In an interactive terminal,
/// let the user cancel; otherwise (piped or `--force`) the warning is advisory.
fn warn_about_secrets(command: &str, force: bool) -> Result<()> {
    use std::io::IsTerminal;
    let found = crate::secrets::scan(command);
    if found.is_empty() {
        return Ok(());
    }
    eprintln!("warning: this command may contain a secret ({})", found.join(", "));
    if force || !std::io::stdin().is_terminal() {
        return Ok(());
    }
    if !prompt_yes("save anyway? [y/N] ")? {
        anyhow::bail!("not saved");
    }
    Ok(())
}

fn prompt_yes(question: &str) -> Result<bool> {
    use std::io::Write;
    eprint!("{question}");
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes" | "Yes"))
}

fn print_row(m: &CommandMemory) {
    let draft = if m.is_draft() { " *" } else { "" };
    let desc = match m.description.as_deref().filter(|d| !d.is_empty()) {
        Some(d) => format!("  — {d}"),
        None => String::new(),
    };
    let tags = if m.tags.is_empty() {
        String::new()
    } else {
        format!("  [{}]", m.tags.join(", "))
    };
    println!("{:>4}{draft}  {}{desc}{tags}", m.id, m.command);
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
