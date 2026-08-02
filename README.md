<div align="center">

<img src="assets/mascot.svg" width="164" alt="recall — a friendly elephant, because it never forgets" />

# recall

**Remember the commands you work out once — recall them by intent.**

<a href="#the-loop">How it works</a> ·
<a href="#the-picker">The picker</a> ·
<a href="#save-from-anywhere">Save from anywhere</a> ·
<a href="#install">Install</a>

</div>

---

You spend twenty minutes getting the `ffmpeg` flags right, or the `kubectl` one‑liner that fixed prod. A month later it's gone — buried in scrollback, re‑derived from scratch.

**recall** keeps it, with a note on *why* it mattered, and hands it back the moment you need it — found by what it *did*, not by exact syntax. It's a small, local, curated library: the commands worth keeping, searched by meaning.

## The loop

The whole tool is one habit and two keystrokes:

> **run a command** → it works → <kbd>Alt</kbd>&nbsp;+&nbsp;<kbd>S</kbd> *keep it (jot the why)* → **forget the syntax** → <kbd>Alt</kbd>&nbsp;+&nbsp;<kbd>R</kbd> *recall it by intent*

That's it. Save while it's fresh, recall when it's needed — without ever leaving your prompt.

## What makes it nice

- <kbd>Alt</kbd>&nbsp;+&nbsp;<kbd>S</kbd> **— save without breaking flow.** The command you just ran becomes a memory in one chord. No retyping, no switching to a notes app.
- <kbd>Alt</kbd>&nbsp;+&nbsp;<kbd>R</kbd> **— recall onto your prompt.** Type what you remember, press Enter, and the command lands on your command line, ready to run or edit. It never executes on its own.
- **Search by intent, not syntax.** `docker disk cleanup` surfaces the command even when those words aren't in it. Typos and whole sentences are fine.
- **The right one floats up.** Ranked by what you reuse most, most recently — usually already at the top before you finish typing.
- **Curate in place.** See a command's *why*, tags, and usage at a glance; edit, delete (with undo), or triage drafts — all inside the picker.
- **Yours, and quiet.** One local SQLite file. No account, no network, no telemetry.

## The picker

Press <kbd>Alt</kbd>&nbsp;+&nbsp;<kbd>R</kbd> (or just run `recall`):

```text
recall — ↑/↓ move · ⏎ print · ^o edit · ^x delete · ^d drafts · esc quit
┌ recall ─────────────────────────┐┌ details ───────────────────────┐
│ search: disk                    ││ docker system prune -af         │
├ 1/2 ─────────────────────────────┤│   --volumes                     │
│▌ ● docker system prune -af ...  ││                                 │
│  ○ du -sh * | sort -h           ││ reclaim disk space by removing  │
│                                 ││ unused docker data              │
│                                 ││ tags: docker, cleanup           │
│                                 ││ used 7× · last used 2d ago      │
└──────────────────────────────────┘└─────────────────────────────────┘
```

Matched characters highlight as you type · `●` / `○` mark curated vs. draft · the pane
collapses to a compact strip on narrow terminals. Readline editing works in every field
(<kbd>Ctrl</kbd>+<kbd>A</kbd> / <kbd>Ctrl</kbd>+<kbd>E</kbd>, <kbd>Ctrl</kbd>+<kbd>W</kbd>, word motions), <kbd>Alt</kbd>+<kbd>Enter</kbd> adds a line to a command, and <kbd>Ctrl</kbd>+<kbd>Z</kbd> undoes a delete.

## Save from anywhere

- **The command you just ran** — <kbd>Alt</kbd>&nbsp;+&nbsp;<kbd>S</kbd>. A one‑line form opens pre‑filled; add the *why* and press Enter. (Skip the note to stash a quick draft and annotate it later.)
- **From scratch** — `recall add` opens the capture form: **command**, **why**, **tags** (the tags field even suggests ones you've used before).
- **From your shell history** — `recall history` browses your past zsh / bash commands; fuzzy‑find, and promote any into a memory.
- **From anywhere else** — point it at *any* command that prints one command per line:

  ```bash
  recall history --file <(your-command-that-lists-commands)
  ```

  Pull items in from another shell, a project runbook, a curated list — whatever you've got.

## Install

```bash
# Homebrew — macOS & Linux
brew install void-restack/recall/recall

# …or from source (needs Rust 1.89+)
cargo install --git https://github.com/void-restack/recall
```

Prebuilt binaries (`.tar.gz`, `.deb`, `.rpm`) are attached to every [release](https://github.com/void-restack/recall/releases).

## Shell integration

The keybindings and last‑command capture come from one line in your shell rc:

```bash
eval "$(recall init zsh --keys)"     # ~/.zshrc
eval "$(recall init bash --keys)"    # ~/.bashrc
```

`--keys` binds <kbd>Alt</kbd>+<kbd>R</kbd> (recall) and <kbd>Alt</kbd>+<kbd>S</kbd> (save the last command). They're opt‑in, configurable (`--recall-key` / `--save-key`), and won't override a key you've already bound.

> On macOS, set your terminal to send **Option as Meta / Esc+** so `Alt+` combos reach the shell (Terminal.app: *Use Option as Meta key*; iTerm2 / kitty have an equivalent).

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

Design decisions live in [`docs/adr/`](docs/adr/) and the domain glossary in [`CONTEXT.md`](CONTEXT.md). The fuzzy backend is touched only in `search.rs`, so swapping matchers is a one‑file change.

## Data & privacy

Everything is a single SQLite file (`~/.local/share/recall/recall.db` on Linux,
`~/Library/Application Support/recall/recall.db` on macOS), created with user‑only
permissions. Selecting a command only *prints* it — nothing runs on its own — and saving
warns when a command looks like it holds a secret. `recall export` / `recall import`
round‑trip the whole collection through JSONL any time.

## License

[MIT](LICENSE)
