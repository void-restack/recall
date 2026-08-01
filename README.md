# recall

> Remember the commands you work out once.

`recall` is a fast, local command memory for your terminal. When you finally nail the `ffmpeg` flags, the `kubectl` one-liner that fixed prod, or that `awk` incantation — save it in one keystroke with a note on *why* it mattered, then pull it back later by whatever you remember: part of the command, the problem it solved, or a tool involved. No more re-deriving the same command every few months or digging through scrollback.

It's a curated library — you keep only the commands worth keeping, and find them by intent instead of exact syntax.

```console
$ recall add 'docker system prune -af --volumes' -d 'reclaim docker disk space' -t docker -t cleanup
$ recall search reclaim disk docker
   1  docker system prune -af --volumes  — reclaim docker disk space  [docker, cleanup]
```

## Why it makes you faster

- **Capture without breaking flow.** One keystroke (`Alt+S`) turns the command you just ran into a saved memory — no retyping, no switching to a notes app.
- **Find by intent, not syntax.** Search `docker disk cleanup` and it surfaces even when those words aren't in the command. Typos and whole sentences are fine, and the commands you reuse most float to the top.
- **Reuse instantly.** `Alt+R` opens a picker and drops the command you choose straight onto your prompt — ready to run or edit. It never executes on its own.
- **Curate as you go.** See a command's *why*, tags, and usage at a glance, and edit or delete it right in the picker.
- **Instant and private.** Everything is a single local SQLite file — no account, no network, no telemetry.

## The interactive UI

Run `recall` with no arguments to open the picker — search on the left, full details on the right:

```text
recall — ↑/↓ move · enter print · ^e edit · ^x delete · esc cancel
┌ search: disk ──────────────────┐┌ details ───────────────────────┐
│▌ docker system prune -af ...    ││ docker system prune -af         │
│  du -sh * | sort -h             ││   --volumes                     │
│                                 ││                                 │
│                                 ││ reclaim disk space by removing  │
│                                 ││ unused docker data              │
│                                 ││                                 │
│                                 ││ tags: docker, cleanup           │
│                                 ││ used 7 times                    │
└─────────────────────────────────┘└─────────────────────────────────┘
```

`recall add` opens a capture form — **command**, **why**, and **tags** fields you tab through. `recall history` browses your past shell commands and promotes any of them into a memory through that same form.

## Install

Requires Rust 1.89+.

```bash
git clone https://github.com/void-restack/recall
cd recall
cargo install --path .
```

## Commands

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

Keep separate collections (work, personal, a project) with `--db <path>` on any command, or the `RECALL_DB` environment variable.

## Shell integration

Add the hook so `recall add --last` can grab the command you just ran, and optionally bind keys:

```bash
# zsh — in ~/.zshrc
eval "$(recall init zsh --keys)"

# bash — in ~/.bashrc
eval "$(recall init bash --keys)"
```

`--keys` binds **Alt+R** (recall a command into your prompt) and **Alt+S** (save the last command). The bindings are opt-in, configurable (`--recall-key` / `--save-key`), and won't override a key you've already bound. Some terminals need Option/Alt set as a Meta/Escape key for `Alt+` combos to reach the shell.

## Principles

- **Local-first and offline.** One SQLite file on your machine (`~/.local/share/recall/` on Linux, `~/Library/Application Support/recall/` on macOS). No account, no network, no telemetry.
- **Safe by default.** Selecting a command prints it — nothing runs on its own. Saving warns when a command looks like it holds a secret, and the database is created with user-only permissions.
- **Yours to keep.** `recall export` / `recall import` round-trip the whole collection through JSONL any time.

## Status

Usable every day: capture (form, `--last`, history promotion), intent search with usage ranking, the interactive picker with in-place edit and delete, secret warnings, JSONL import/export, and configurable shell keybindings. Coming next: Fish support, broader tests, and packaging. Not yet published to a package manager.

## License

Not yet chosen — a permissive OSI license will be added before the first release.
