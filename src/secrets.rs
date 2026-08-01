use std::sync::LazyLock;

use regex::Regex;

// Advisory only: pattern-matching can't prove a command is safe or unsafe (see the
// brief's §14.2). Patterns are kept conservative to avoid crying wolf on normal commands.
static PATTERNS: LazyLock<Vec<(&'static str, Regex)>> = LazyLock::new(|| {
    let compile = |label: &'static str, pat: &str| (label, Regex::new(pat).expect("valid pattern"));
    vec![
        compile("AWS access key", r"AKIA[0-9A-Z]{16}"),
        compile("GitHub token", r"gh[pousr]_[A-Za-z0-9]{20,}"),
        compile("Slack token", r"xox[baprs]-[A-Za-z0-9-]{10,}"),
        compile("private key block", r"-----BEGIN [A-Z ]*PRIVATE KEY-----"),
        compile("bearer token", r"(?i)bearer\s+[A-Za-z0-9._\-]{12,}"),
        compile("credential in URL", r"://[^/\s:@]+:[^/\s@]+@"),
        compile(
            "inline secret",
            r"(?i)(password|passwd|pwd|secret|token|api[_-]?key|access[_-]?key)\s*[=:]\s*\S{6,}",
        ),
    ]
});

/// Labels of the secret-like patterns found in `command`, if any.
pub fn scan(command: &str) -> Vec<&'static str> {
    PATTERNS
        .iter()
        .filter(|(_, re)| re.is_match(command))
        .map(|(label, _)| *label)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_common_secrets() {
        assert!(!scan("aws configure set aws_access_key_id AKIAIOSFODNN7EXAMPLE").is_empty());
        assert!(!scan("curl -H 'Authorization: Bearer sk-abc123def456ghi789xyz'").is_empty());
        assert!(!scan("psql postgres://user:s3cretpw@db.example.com/app").is_empty());
        assert!(!scan("export GITHUB_TOKEN=ghp_abcdefghijklmnopqrstuvwxyz0123456789").is_empty());
        assert!(!scan("mysql --password=hunter2please").is_empty());
    }

    #[test]
    fn leaves_normal_commands_alone() {
        assert!(scan("docker ps -a").is_empty());
        assert!(scan("kubectl get pods --field-selector=status.phase=Failed").is_empty());
        assert!(scan("git commit -m 'fix the login token bug'").is_empty());
        assert!(scan("ffmpeg -i in.mov -c:v libx264 out.mp4").is_empty());
    }
}
