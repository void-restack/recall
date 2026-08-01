# recall

> Remember the commands you work out once — recall them by intent.

`recall` is a fast, local command memory for your terminal. When you finally nail
the `ffmpeg` flags or the `kubectl` one-liner that fixed prod, save it in one
keystroke with a note on *why* it mattered — then pull it back later by whatever
you remember: part of the command, the problem it solved, or a tool involved.

It's a **curated** library, not a bigger history file: you keep only the commands
worth keeping, and find them by meaning instead of exact syntax.

```console
$ recall add 'docker system prune -af --volumes' -d 'reclaim docker disk space' -t docker
$ recall search reclaim disk docker
   1  docker system prune -af --volumes  — reclaim docker disk space  [docker]
```

## What's different

- **Search by intent, not syntax.** `docker disk cleanup` finds the command even
  when those words aren't in it. Typos and whole sentences are fine.
- **Capture without breaking flow.** `Alt+S` turns the command you just ran into a
  saved memory — no retyping, no notes app.
- **Reuse safely.** `Alt+R` drops the chosen command onto your prompt, ready to run
  or edit. It never executes on its own.
- **The right command floats up.** Ranked by frecency — what you reuse most, most
  recently — so it's usually already at the top before you type.
- **Curate in place.** A polished inline picker with live match highlighting,
  edit/delete, drafts triage, and one-key undo.
- **Local and private.** One SQLite file on your machine. No account, no network,
  no telemetry.

## The picker

Run `recall` with no arguments:

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

Matched characters highlight as you type; `●`/`○` mark curated vs. draft; the pane
collapses to a compact strip on narrow terminals. Readline editing (`Ctrl-A/E`,
`Ctrl-W`, word motions) works in every field, `Alt-Enter` adds a line to a command,
and `Ctrl-Z` undoes a delete. `recall add` opens the same form to capture a command;
`recall history` promotes a past shell command into a memory.

## Install

Requires Rust 1.89+.

```bash
# straight from the repo
cargo install --git https://github.com/void-restack/recall

# or from a clone
git clone https://github.com/void-restack/recall && cd recall
cargo install --path .
```

## Usage

```text
recall                                       Open the interactive picker
recall add [<cmd>] [-d <desc>] [-t <tag> …]  Save a command (opens the form if run bare)
recall add --last                            Save the command you just ran
recall history                               Browse shell history and promote a command
recall search <words…>                       Fuzzy, typo-tolerant search by intent
recall list [--drafts]                       List everything (or only un-annotated drafts)
recall get <id>                              Print a command to stdout (counts as a use)
recall edit <id> [-c <cmd>] [-d <desc>] …    Change a command, description, or tags
recall delete <id> [-y]                      Delete (asks first unless -y)
recall export | import <file>                Back up / restore as JSONL
recall init <bash|zsh> [--keys]              Print shell integration
```

Keep separate collections (work, personal, a project) with `--db <path>` on any
command, or the `RECALL_DB` environment variable.

### Shell integration

Add the hook so `recall add --last` can grab the command you just ran, and
optionally bind keys:

```bash
eval "$(recall init zsh --keys)"    # ~/.zshrc
eval "$(recall init bash --keys)"   # ~/.bashrc
```

`--keys` binds **Alt+R** (recall into your prompt) and **Alt+S** (save the last
command). Bindings are opt-in, configurable (`--recall-key` / `--save-key`), and
won't override a key you've already bound.

## Development

```bash
git clone https://github.com/void-restack/recall && cd recall
cargo build
cargo test
cargo fmt
cargo clippy --all-targets
```

The code is layered so each concern stays swappable:

| Layer | Modules |
| --- | --- |
| Interface (CLI args) | `cli.rs`, `main.rs` |
| Application (commands) | `commands.rs` |
| Repository (SQLite) | `store.rs` |
| Search (fuzzy matcher) | `search.rs` |
| Inline TUI | `tui.rs`, `line_editor.rs`, `theme.rs` |
| Supporting | `memory.rs`, `secrets.rs`, `history.rs`, `shell.rs`, `paths.rs` |

Design decisions live in [`docs/adr/`](docs/adr/) and the domain glossary in
[`CONTEXT.md`](CONTEXT.md). The fuzzy backend
([frizbee](https://crates.io/crates/frizbee)) is touched only in `search.rs`, so
swapping matchers is a one-file change.

## Data & privacy

Everything is a single SQLite file (`~/.local/share/recall/recall.db` on Linux,
`~/Library/Application Support/recall/recall.db` on macOS), created with user-only
permissions. Selecting a command only *prints* it — nothing runs on its own — and
saving warns when a command looks like it holds a secret. `recall export` /
`recall import` round-trip the whole collection through JSONL at any time.

## License

[MIT](LICENSE)
