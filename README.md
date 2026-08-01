# recall

> Remember the commands you figure out once.

`recall` is a local-first command memory for your terminal. When you finally land the `ffmpeg` flags or the `kubectl` one-liner that fixed prod, save it in one keystroke — with a note on *why* it mattered — and find it again later by whatever fragment you remember: part of the command, the problem it solved, or a tool involved.

It's not shell history. History records everything you ran; `recall` keeps only what you deliberately choose to remember, and searches it by intent instead of exact syntax.

```console
$ recall add 'docker system prune -af --volumes' -d 'reclaim docker disk space' -t docker -t cleanup
$ recall search reclaim disk docker
   1  docker system prune -af --volumes  — reclaim docker disk space  [docker, cleanup]
```

## Install

Requires Rust 1.89+.

```bash
git clone https://github.com/void-restack/recall
cd recall
cargo install --path .
```

## Use

```text
recall add <cmd> [-d <desc>] [-t <tag> …]   Save a command (a Draft if you skip -d)
recall add --last [-d <desc>] [-t <tag> …]  Save the previous command (needs the shell hook)
recall search <words…>                      Fuzzy, typo-tolerant search by intent
recall list                                 List everything, newest first
recall get <id>                             Print a command to stdout (counts as a use)
recall edit <id> [-c <cmd>] [-d <desc>] …   Change a command, description, or tags
recall delete <id> [-y]                      Delete (asks first unless -y)
recall export                               Dump all memories as JSONL
recall init <bash|zsh>                       Print shell integration for --last
```

Search matches the words you type against each memory's command, description, and tags — so `docker disk cleanup` finds it even if those words aren't in the command itself. Filler words in a longer query are ignored.

## Save the previous command

Enable the shell hook so `recall add --last` can grab the command you just ran:

```bash
# zsh — add to ~/.zshrc
eval "$(recall init zsh)"

# bash — add to ~/.bashrc
eval "$(recall init bash)"
```

Then, right after a command you want to keep:

```bash
recall add --last -d 'why this mattered' -t sometag
```

## Principles

- **Local-first and offline.** Everything lives in a single SQLite file on your machine (`~/.local/share/recall/` on Linux, `~/Library/Application Support/recall/` on macOS). No account, no network, no telemetry.
- **Safe by default.** `get` and search print the command — nothing runs on its own. The database is created with user-only permissions.
- **Yours to keep.** `recall export` dumps everything to JSONL any time.

## Status

Early but usable: the capture → search → reuse loop works today. Still to come: an interactive picker, usage-based ranking, secret warnings, and JSONL import. Not yet published to a package manager.

## License

Not yet chosen — a permissive OSI license will be added before the first release.
