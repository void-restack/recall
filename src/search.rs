use frizbee::{Config, Matcher, Pattern};

use crate::memory::CommandMemory;

/// Fuzzy, typo-tolerant search over each Memory's command, description, and tags,
/// returning matches best-first. The only place that touches the fuzzy backend
/// (frizbee), so swapping matchers stays a one-file change.
pub fn search<'a>(query: &str, memories: &'a [CommandMemory], limit: usize) -> Vec<&'a CommandMemory> {
    let query = query.trim();
    if query.is_empty() || memories.is_empty() {
        return Vec::new();
    }

    // One typo per four query characters, so `dokcer` still finds `docker`.
    let patterns: Vec<Pattern> = Pattern::parse_query(query)
        .into_iter()
        .map(|p| {
            let max_typos = (p.needle.len() / 4) as u16;
            p.max_typos(Some(max_typos))
        })
        .collect();
    if patterns.is_empty() {
        return Vec::new();
    }

    let haystacks: Vec<String> = memories.iter().map(haystack_for).collect();
    let mut matcher = Matcher::from_patterns(&patterns, &Config::default());

    let mut hits: Vec<&CommandMemory> = matcher
        .match_list(&haystacks)
        .into_iter()
        .map(|m| &memories[m.index as usize])
        .collect();
    hits.truncate(limit);
    hits
}

fn haystack_for(m: &CommandMemory) -> String {
    let mut text = m.command.clone();
    if let Some(desc) = &m.description {
        text.push(' ');
        text.push_str(desc);
    }
    if !m.tags.is_empty() {
        text.push(' ');
        text.push_str(&m.tags.join(" "));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem(id: i64, command: &str, description: Option<&str>, tags: &[&str]) -> CommandMemory {
        CommandMemory {
            id,
            command: command.into(),
            description: description.map(Into::into),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            created_at: 0,
            updated_at: 0,
            use_count: 0,
        }
    }

    fn corpus() -> Vec<CommandMemory> {
        vec![
            mem(1, "docker system prune -af --volumes", Some("reclaim docker disk space"), &["docker", "cleanup"]),
            mem(2, "git reflog --date=iso", Some("recover a lost commit"), &["git"]),
            mem(3, "kubectl get pods --field-selector=status.phase=Failed", Some("find failing pods"), &["kubernetes"]),
        ]
    }

    #[test]
    fn finds_by_purpose_words_not_in_the_command() {
        let c = corpus();
        let hits = search("docker disk cleanup", &c, 10);
        assert_eq!(hits.first().map(|m| m.id), Some(1));
    }

    #[test]
    fn tolerates_a_typo() {
        let c = corpus();
        let hits = search("dokcer", &c, 10);
        assert_eq!(hits.first().map(|m| m.id), Some(1));
    }

    #[test]
    fn no_matches_returns_empty() {
        let c = corpus();
        assert!(search("zzzznomatch", &c, 10).is_empty());
    }
}
