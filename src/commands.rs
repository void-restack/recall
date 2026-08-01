use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};

use crate::cli::{
    AddArgs, DeleteArgs, EditArgs, GetArgs, HistoryArgs, ImportArgs, InitArgs, ListArgs,
    SearchArgs, Shell,
};
use crate::memory::{self, CommandMemory, ImportRecord, NewMemory};
use crate::store::Store;
use crate::{history, paths, shell};

pub fn add(args: AddArgs) -> Result<()> {
    use std::io::IsTerminal;
    let interactive = std::io::stderr().is_terminal();
    let no_annotations = args.description.is_none() && args.tag.is_empty();

    // With no command argument and no annotation flags, an interactive terminal
    // gets the capture form; --last pre-fills it with the previous command.
    if interactive && no_annotations && args.command.is_none() {
        let initial = if args.last {
            read_last_command()?
        } else {
            String::new()
        };
        return add_via_form(initial, args.force);
    }

    let command = resolve_command(args.command, args.last)?;
    warn_about_secrets(&command, args.force)?;
    save_new(
        command,
        clean_description(args.description),
        memory::normalize_tags(args.tag),
    )
}

fn add_via_form(initial: String, force: bool) -> Result<()> {
    let Some(form) = crate::tui::add_form(&initial, "", "")? else {
        eprintln!("cancelled");
        return Ok(());
    };
    warn_about_secrets(&form.command, force)?;
    let tags = memory::normalize_tags(split_tags(&form.tags));
    save_new(
        form.command,
        clean_description(Some(form.description)),
        tags,
    )
}

fn split_tags(raw: &str) -> Vec<String> {
    raw.split([',', ' '])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn save_new(command: String, description: Option<String>, tags: Vec<String>) -> Result<()> {
    let store = Store::open()?;
    let duplicates = store.ids_with_command(&command)?;
    let new = NewMemory {
        command,
        description,
        tags,
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
        Shell::Zsh => shell::zsh(
            &last_file,
            args.keys,
            args.recall_key.as_deref().unwrap_or("^[r"),
            args.save_key.as_deref().unwrap_or("^[s"),
        ),
        Shell::Bash => shell::bash(
            &last_file,
            args.keys,
            args.recall_key.as_deref().unwrap_or(r"\er"),
            args.save_key.as_deref().unwrap_or(r"\es"),
        ),
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
    let backend = PickerBackend { store: &store };
    if let Some(chosen) = crate::tui::run(&backend, memories, now_millis())? {
        store.record_use(chosen.id, now_millis())?;
        println!("{}", chosen.command);
    }
    Ok(())
}

/// Adapts the Store to the picker's persistence needs, so edits and deletes made
/// inside the live viewport hit the database without leaving the TUI layer.
struct PickerBackend<'a> {
    store: &'a Store,
}

impl crate::tui::PickerStore for PickerBackend<'_> {
    fn reload(&self) -> Result<Vec<CommandMemory>> {
        self.store.list()
    }

    fn save_edit(&self, id: i64, form: crate::tui::AddForm) -> Result<()> {
        let mut m = self
            .store
            .get(id)?
            .ok_or_else(|| anyhow!("no memory #{id}"))?;
        m.command = form.command;
        m.description = clean_description(Some(form.description));
        m.tags = memory::normalize_tags(split_tags(&form.tags));
        self.store.update(&m, now_millis())
    }

    fn delete(&self, id: i64) -> Result<()> {
        self.store.delete(id)?;
        Ok(())
    }
}

pub fn list(args: ListArgs) -> Result<()> {
    let store = Store::open()?;
    let mut memories = store.list()?;
    if args.drafts {
        memories.retain(CommandMemory::is_draft);
    }
    if memories.is_empty() {
        let message = if args.drafts {
            "no drafts — everything is annotated"
        } else {
            "no memories yet — capture one with `recall add`"
        };
        eprintln!("{message}");
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
        let record: ImportRecord = serde_json::from_str(&line)
            .with_context(|| format!("line {}: invalid record", n + 1))?;
        if record.command.trim().is_empty() {
            anyhow::bail!("line {}: record has an empty command", n + 1);
        }
        records.push(record);
    }

    let count = Store::open()?.import_all(&records, now_millis())?;
    eprintln!("imported {count} memories");
    Ok(())
}

pub fn history(args: HistoryArgs) -> Result<()> {
    use std::io::IsTerminal;
    if !std::io::stderr().is_terminal() {
        anyhow::bail!("not a terminal — the history picker needs an interactive shell");
    }
    let entries = history::recent(args.file, args.limit)?;
    if entries.is_empty() {
        eprintln!("no shell history found to promote");
        return Ok(());
    }
    let Some(index) = crate::tui::history_picker(&entries)? else {
        eprintln!("cancelled");
        return Ok(());
    };
    let Some(form) = crate::tui::add_form(&entries[index], "", "")? else {
        eprintln!("cancelled");
        return Ok(());
    };
    warn_about_secrets(&form.command, false)?;
    let tags = memory::normalize_tags(split_tags(&form.tags));
    save_new(
        form.command,
        clean_description(Some(form.description)),
        tags,
    )
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
    eprintln!(
        "warning: this command may contain a secret ({})",
        found.join(", ")
    );
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
    println!("{:>4}{draft}  {}{desc}{tags}", m.id, one_line(&m.command));
}

/// Collapse a multi-line command to its first line plus a `⏎N` marker, so a list
/// row never spills raw newlines. `get`/the picker still emit the command verbatim.
fn one_line(command: &str) -> String {
    match command.split_once('\n') {
        Some((first, rest)) => format!("{first} ⏎{}", rest.lines().count()),
        None => command.to_string(),
    }
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
