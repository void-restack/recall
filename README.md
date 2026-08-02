<div align="center">

<img src="assets/mascot.svg" width="140" alt="recall — a friendly elephant, because it never forgets" />

# recall

**Remember the commands you work out once — recall them by intent.**

[The loop](#the-loop) · [The picker](#the-picker) · [Save from anywhere](#save-from-anywhere) · [Install](#install)

</div>

---

Twenty minutes on `ffmpeg` flags. The `kubectl` one-liner that fixed prod. A month later — gone.

**recall** keeps it, notes *why* it mattered, and hands it back when you need it: found by what it *did*, not by exact syntax. A small, local, curated library of the commands worth keeping, searched by meaning.

## The loop

One habit, two keystrokes:

> run a command → it works → <kbd>Alt</kbd>+<kbd>S</kbd> _keep it_ → forget the syntax → <kbd>Alt</kbd>+<kbd>R</kbd> _recall it by intent_

Save while it's fresh. Recall when it's needed. Never leave your prompt.

## What makes it nice

- **<kbd>Alt</kbd>+<kbd>S</kbd> — save without breaking flow.** The command you just ran becomes a memory in one chord. No retyping, no switching apps.
- **<kbd>Alt</kbd>+<kbd>R</kbd> — recall onto your prompt.** Type what you remember, press Enter, and the command lands on your command line — ready to run or edit, never executed on its own.
- **Search by intent, not syntax.** `docker disk cleanup` finds the command even when those words aren't in it. Typos and whole sentences are fine.
- **The right one floats up.** Ranked by what you reuse most and most recently — usually at the top before you finish typing.
- **Curate in place.** Why, tags, and usage at a glance; edit, delete (with undo), or triage drafts — all inside the picker.
- **Yours, and quiet.** One local SQLite file. No account, no network, no telemetry.

## The picker

<kbd>Alt</kbd>+<kbd>R</kbd> (or just `recall`) opens the picker:

```text
recall — ↑/↓ move · ⏎ print · ^o edit · ^x delete · ^d drafts · esc quit
┌ recall ──────────────┐┌ details ─────────────────┐
│ search: disk         ││ docker system prune -af  │
├──────────────────────┤│  --volumes               │
│▌● docker system …    ││                          │
│  ○ du -sh * | sort…  ││ reclaim disk space by    │
│                      ││ removing unused docker   │
│                      ││ tags: docker, cleanup    │
│                      ││ used 7× · last 2d ago    │
└──────────────────────┘└──────────────────────────┘
```

Matched characters highlight as you type · `●`/`○` mark curated vs. draft · the pane collapses to a compact strip on narrow terminals. Readline editing works in every field (<kbd>Ctrl</kbd>+<kbd>A</kbd>/<kbd>E</kbd>, <kbd>Ctrl</kbd>+<kbd>W</kbd>, word motions), <kbd>Alt</kbd>+<kbd>Enter</kbd> adds a line to a command, and <kbd>Ctrl</kbd>+<kbd>Z</kbd> undoes a delete.

## Save from anywhere

- **The command you just ran** — <kbd>Alt</kbd>+<kbd>S</kbd>. A one-line form opens pre-filled; add the *why* and press Enter. Skip the note to stash a quick draft and annotate it later.
- **From scratch** — `recall add` opens the capture form: command, why, tags (with suggestions from tags you've used before).
- **From shell history** — `recall history` browses your past zsh/bash commands; fuzzy-find and promote any into a memory.
- **From anywhere else** — point it at anything that prints one command per line:

  ```bash
  recall history --file <(your-command-that-lists-commands)
  ```

  Pull items in from another shell, a project runbook, or a curated list.

## Install

```bash
# Homebrew — macOS & Linux
brew install void-restack/tap/recall

# …or from source (needs Rust 1.89+)
cargo install --git https://github.com/void-restack/recall
```

Prebuilt binaries (`.tar.gz`, `.deb`, `.rpm`) are attached to every [release](https://github.com/void-restack/recall/releases).

## Shell integration

The keybindings and last-command capture come from one line in your shell rc:

```bash
eval "$(recall init zsh --keys)"     # ~/.zshrc
eval "$(recall init bash --keys)"    # ~/.bashrc
```

`--keys` binds <kbd>Alt</kbd>+<kbd>R</kbd> (recall) and <kbd>Alt</kbd>+<kbd>S</kbd> (save the last command). They're opt-in, configurable (`--recall-key` / `--save-key`), and won't override a key you've already bound.

> On macOS, set your terminal to send **Option as Meta / Esc+** so `Alt`+ combos reach the shell (Terminal.app: *Use Option as Meta key*; iTerm2 / kitty have an equivalent).

<details>
<summary><b>Command reference</b> — everything the keys do, as plain commands</summary>

```text
recall                                       open the interactive picker
recall add [<cmd>] [-d <why>] [-t <tag> …]   save a command (opens the form if run bare)
recall add --last                            save the command you just ran
recall history                               browse shell history and promote a command
recall search <words…>                       fuzzy, typo-tolerant search by intent
recall list [--drafts]                       list everything (or only un-annotated drafts)
recall get <id>                              print a command to stdout (counts as a use)
recall edit <id> [-c <cmd>] [-d <why>] …     change a command, description, or tags
recall delete <id> [-y]                      delete (asks first unless -y)
recall export | import <file>                back up / restore as JSONL
recall init <bash|zsh> [--keys]              print shell integration
```

Keep separate collections (work, personal, a project) with `--db <path>` on any command,
or the `RECALL_DB` environment variable.

</details>

## Development

```bash
git clone https://github.com/void-restack/recall && cd recall
cargo build && cargo test
cargo fmt && cargo clippy --all-targets
```

Layered so each concern stays swappable:

| Layer | Modules |
| --- | --- |
| Interface (CLI args) | `cli.rs`, `main.rs` |
| Application (commands) | `commands.rs` |
| Repository (SQLite) | `store.rs` |
| Search (fuzzy matcher) | `search.rs` |
| Inline TUI | `tui.rs`, `line_editor.rs`, `theme.rs` |

The fuzzy backend is touched only in `search.rs`, so swapping matchers is a one-file change.

## Data & privacy

Everything lives in a single SQLite file (`~/.local/share/recall/recall.db` on Linux,
`~/Library/Application Support/recall/recall.db` on macOS), created with user-only
permissions. Selecting a command only *prints* it — nothing runs on its own — and saving
warns when a command looks like it holds a secret. `recall export` / `import` round-trip
the whole collection as JSONL any time.

## License

[MIT](LICENSE)
