use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Recent shell commands, newest first, adjacent duplicates collapsed and recall's
/// own invocations skipped. Best-effort across zsh and bash history formats.
pub fn recent(file: Option<PathBuf>, limit: usize) -> Result<Vec<String>> {
    let path = resolve(file)?;
    let bytes = std::fs::read(&path)
        .with_context(|| format!("reading history {}", path.display()))?;
    // zsh stores bytes >= 0x80 metafied, and the file need not be valid UTF-8.
    let decoded = unmetafy(&bytes);
    let text = String::from_utf8_lossy(&decoded);
    Ok(extract(&text, limit))
}

/// Reverse zsh's history metafication: a `0x83` byte escapes the next byte as `byte ^ 0x20`.
fn unmetafy(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut iter = bytes.iter().copied();
    while let Some(byte) = iter.next() {
        if byte == 0x83 {
            if let Some(escaped) = iter.next() {
                out.push(escaped ^ 0x20);
            }
        } else {
            out.push(byte);
        }
    }
    out
}

fn resolve(file: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(file) = file {
        return Ok(file);
    }
    if let Ok(histfile) = std::env::var("HISTFILE") {
        let path = PathBuf::from(histfile);
        if path.is_file() {
            return Ok(path);
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        for name in [".zsh_history", ".bash_history"] {
            let path = Path::new(&home).join(name);
            if path.is_file() {
                return Ok(path);
            }
        }
    }
    anyhow::bail!("could not find a shell history file — pass --file <PATH>")
}

fn extract(text: &str, limit: usize) -> Vec<String> {
    let mut commands: Vec<String> = Vec::new();
    for raw in text.lines() {
        let Some(command) = parse_line(raw) else {
            continue;
        };
        let command = command.trim();
        if command.is_empty() || command == "recall" || command.starts_with("recall ") {
            continue;
        }
        if commands.last().map(String::as_str) == Some(command) {
            continue; // collapse adjacent duplicates
        }
        commands.push(command.to_string());
    }
    commands.reverse(); // newest first
    commands.truncate(limit);
    commands
}

/// Extract the command from a history line, or `None` for metadata / blank lines.
fn parse_line(line: &str) -> Option<&str> {
    let line = line.trim_end();
    if line.is_empty() {
        return None;
    }
    // zsh extended history: ": <start>:<duration>;<command>"
    if let Some(rest) = line.strip_prefix(": ")
        && let Some((_, command)) = rest.split_once(';')
    {
        return Some(command);
    }
    // bash timestamp line: "#<epoch>"
    if let Some(rest) = line.strip_prefix('#')
        && !rest.is_empty()
        && rest.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    Some(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_common_formats() {
        assert_eq!(parse_line(": 1700000000:0;docker ps"), Some("docker ps"));
        assert_eq!(parse_line("git status"), Some("git status"));
        assert_eq!(parse_line("#1700000000"), None);
        assert_eq!(parse_line("   "), None);
    }

    #[test]
    fn extract_reverses_dedupes_and_skips_recall() {
        let text = ": 1:0;docker ps\n: 2:0;docker ps\ngit status\nrecall add foo\n#1700000000\nls\n";
        assert_eq!(extract(text, 10), vec!["ls", "git status", "docker ps"]);
    }

    #[test]
    fn extract_respects_the_limit() {
        let text = "one\ntwo\nthree\nfour\n";
        assert_eq!(extract(text, 2), vec!["four", "three"]);
    }

    #[test]
    fn unmetafy_decodes_zsh_meta_bytes() {
        // "é" is 0xC3 0xA9; zsh metafies each high byte as 0x83 then (byte ^ 0x20).
        let meta = [0x83, 0xC3 ^ 0x20, 0x83, 0xA9 ^ 0x20];
        assert_eq!(String::from_utf8(unmetafy(&meta)).unwrap(), "é");
    }

    #[test]
    fn recent_survives_invalid_utf8() {
        use std::io::Write;
        let mut path = std::env::temp_dir();
        path.push(format!("recall-hist-{}.tmp", std::process::id()));
        std::fs::File::create(&path)
            .unwrap()
            .write_all(b": 1:0;docker ps\n\xff\xfe junk\n")
            .unwrap();
        let got = recent(Some(path.clone()), 100);
        std::fs::remove_file(&path).ok();
        assert!(got.unwrap().iter().any(|c| c == "docker ps"));
    }
}
