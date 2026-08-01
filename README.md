# recall

> Remember the commands you figure out once.

`recall` is a local-first command memory for your terminal. When you finally land the `ffmpeg` flags or the `kubectl` one-liner that fixed prod, save it in one keystroke — with a note on *why* it mattered — and find it again later by whatever fragment you remember: part of the command, the problem it solved, or a tool involved.

It's not shell history. History records everything you ran; `recall` keeps only what you deliberately choose to remember, and searches it by intent instead of exact syntax.

```bash
# Save the command you just ran — annotate now or later
recall add --last -d "reclaim docker disk space" -t docker -t cleanup

# Find it later by what it did, not how it was spelled
recall "docker disk cleanup"
```

## Principles

- **Capture has to be effortless.** If saving isn't nearly free, the library stays empty — so it's one keystroke from the command you just ran to a saved memory.
- **Local-first and offline.** Everything lives in a single SQLite file on your machine. No account, no network, no telemetry. Your commands never leave.
- **Safe by default.** Selecting a result prints it — it never runs on its own. Likely secrets are flagged before saving.
- **Yours to keep.** Export the whole collection to JSONL anytime.

## Status

Early development. The design is settled and implementation is just starting, so there's nothing to install yet. Watch the repo to follow along.

## License

Not yet chosen — a permissive OSI license will be added before the first release.
