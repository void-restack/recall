use frizbee::{Config, Matcher};

use crate::memory::CommandMemory;

/// Everyday filler that describes *how you remember* a command rather than the
/// command itself ("the command I ran yesterday that helped me..."). Dropped
/// before matching. Deliberately excludes words that are also real commands or
/// flags (run, find, get, list, show, all) — those stay searchable.
const STOPWORDS: &[&str] = &[
    "a",
    "an",
    "the",
    "and",
    "or",
    "but",
    "of",
    "to",
    "in",
    "on",
    "at",
    "for",
    "with",
    "by",
    "from",
    "as",
    "i",
    "me",
    "my",
    "we",
    "us",
    "our",
    "you",
    "your",
    "it",
    "its",
    "that",
    "this",
    "these",
    "those",
    "there",
    "then",
    "than",
    "so",
    "is",
    "am",
    "are",
    "was",
    "were",
    "be",
    "been",
    "being",
    "do",
    "does",
    "did",
    "have",
    "has",
    "had",
    "can",
    "could",
    "will",
    "would",
    "should",
    "may",
    "might",
    "must",
    "how",
    "what",
    "when",
    "where",
    "why",
    "who",
    "which",
    "about",
    "just",
    "really",
    "actually",
    "yesterday",
    "today",
    "tomorrow",
    "ago",
    "day",
    "week",
    "command",
    "thing",
    "ran",
    "helped",
    "want",
    "wanted",
    "need",
    "needed",
];

/// Build the per-memory search text once. The picker holds onto this across
/// keystrokes instead of rebuilding the whole haystack on every refilter.
pub fn build_haystacks(memories: &[CommandMemory]) -> Vec<String> {
    memories.iter().map(haystack_for).collect()
}

/// Rank memories against pre-built haystacks, best-first, returning their indices
/// into `memories` directly (callers no longer need an id→index lookup).
pub fn ranked_indices(
    query: &str,
    memories: &[CommandMemory],
    haystacks: &[String],
    limit: usize,
) -> Vec<usize> {
    if query.trim().is_empty() || memories.is_empty() {
        return Vec::new();
    }
    let (matched, score) = coverage(query, haystacks);

    // Relevance first (terms matched, then match quality); Curated over Draft, then
    // usage — so a command reused often, and more recently, wins between equal matches.
    let mut ranked: Vec<usize> = (0..memories.len()).filter(|&i| matched[i] > 0).collect();
    ranked.sort_by(|&a, &b| {
        matched[b]
            .cmp(&matched[a])
            .then(score[b].cmp(&score[a]))
            .then(memories[a].is_draft().cmp(&memories[b].is_draft()))
            .then(memories[b].use_count.cmp(&memories[a].use_count))
            .then(memories[b].last_used_at.cmp(&memories[a].last_used_at))
    });
    ranked.truncate(limit);
    ranked
}

/// Fuzzy, typo-tolerant search over each Memory's command, description, and tags,
/// returning matches best-first. The only place that touches the fuzzy backend
/// (frizbee), so swapping matchers stays a one-file change.
pub fn search<'a>(
    query: &str,
    memories: &'a [CommandMemory],
    limit: usize,
) -> Vec<&'a CommandMemory> {
    let haystacks = build_haystacks(memories);
    ranked_indices(query, memories, &haystacks, limit)
        .into_iter()
        .map(|i| &memories[i])
        .collect()
}

/// About 30 days, in milliseconds — the frecency half-life.
const HALF_LIFE_MS: f64 = 30.0 * 86_400.0 * 1000.0;

/// Order for the empty query: by frecency (use count decayed toward the last use),
/// with drafts sunk beneath curated memories to nudge triage. Returns indices.
pub fn frecency_order(memories: &[CommandMemory], now: i64) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..memories.len()).collect();
    idx.sort_by(|&a, &b| {
        let (x, y) = (&memories[a], &memories[b]);
        x.is_draft()
            .cmp(&y.is_draft())
            .then(frecency_score(y, now).total_cmp(&frecency_score(x, now)))
            .then(y.created_at.cmp(&x.created_at))
    });
    idx
}

/// Use count decayed by how long ago the memory was last used (falling back to when
/// it was created). The `+1` keeps never-used memories ordered by recency alone.
fn frecency_score(m: &CommandMemory, now: i64) -> f64 {
    let last = m.last_used_at.unwrap_or(m.created_at);
    let age = (now - last).max(0) as f64;
    let decay = 0.5_f64.powf(age / HALF_LIFE_MS);
    (m.use_count as f64 + 1.0) * decay
}

/// Rank plain lines (e.g. shell history) by the same fuzzy coverage, best first.
pub fn rank_lines(query: &str, lines: &[String], limit: usize) -> Vec<usize> {
    if query.trim().is_empty() {
        return (0..lines.len()).take(limit).collect();
    }
    let (matched, score) = coverage(query, lines);
    let mut ranked: Vec<usize> = (0..lines.len()).filter(|&i| matched[i] > 0).collect();
    ranked.sort_by(|&a, &b| matched[b].cmp(&matched[a]).then(score[b].cmp(&score[a])));
    ranked.truncate(limit);
    ranked
}

/// Char positions in `text` matched by the query's terms, for highlighting a row.
/// Uses the same terms and typo tolerance as ranking, so highlights track hits.
pub fn match_positions(query: &str, text: &str) -> Vec<usize> {
    let mut positions = std::collections::HashSet::new();
    let haystack = [text];
    for term in query_terms(query) {
        let config = Config::default().max_typos(Some((term.len() / 4) as u16));
        let mut matcher = Matcher::new(term.as_str(), &config);
        for m in matcher.match_list_indices(&haystack) {
            // frizbee reports char indices (in reverse); a set makes order moot.
            positions.extend(m.indices.into_iter().map(|i| i as usize));
        }
    }
    let mut out: Vec<usize> = positions.into_iter().collect();
    out.sort_unstable();
    out
}

/// The query terms that matched `haystack`, and the filler dropped before matching —
/// for the picker's "why this matched" transparency line.
pub fn explain_match(query: &str, haystack: &str) -> (Vec<String>, Vec<String>) {
    let raw: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let kept = query_terms(query);
    let dropped: Vec<String> = raw.into_iter().filter(|w| !kept.contains(w)).collect();
    let text = [haystack];
    let matched: Vec<String> = kept
        .into_iter()
        .filter(|term| {
            let config = Config::default().max_typos(Some((term.len() / 4) as u16));
            let mut matcher = Matcher::new(term.as_str(), &config);
            !matcher.match_list(&text).is_empty()
        })
        .collect();
    (matched, dropped)
}

/// For each haystack, count how many query terms matched and sum their scores.
/// An OR/coverage measure, so leftover filler in a sentence can't zero a row out.
fn coverage(query: &str, haystacks: &[String]) -> (Vec<u32>, Vec<u32>) {
    let mut matched = vec![0u32; haystacks.len()];
    let mut score = vec![0u32; haystacks.len()];
    for term in query_terms(query) {
        // One typo per four characters, so `dokcer` still finds `docker`.
        let config = Config::default().max_typos(Some((term.len() / 4) as u16));
        let mut matcher = Matcher::new(term.as_str(), &config);
        for m in matcher.match_list(haystacks) {
            let i = m.index as usize;
            matched[i] += 1;
            score[i] += u32::from(m.score);
        }
    }
    (matched, score)
}

fn query_terms(query: &str) -> Vec<String> {
    let raw: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let kept: Vec<String> = raw
        .iter()
        .filter(|w| w.len() > 1 && !STOPWORDS.contains(&w.as_str()))
        .cloned()
        .collect();
    // If the query was nothing but filler, fall back to whatever was typed.
    if kept.is_empty() { raw } else { kept }
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
            last_used_at: None,
        }
    }

    fn corpus() -> Vec<CommandMemory> {
        vec![
            mem(
                1,
                "docker ps",
                Some("list running (active) docker containers"),
                &["docker", "containers"],
            ),
            mem(
                2,
                "docker ps -a",
                Some("list all docker containers including stopped"),
                &["docker", "containers"],
            ),
            mem(
                3,
                "git reflog --date=iso",
                Some("recover a lost commit"),
                &["git"],
            ),
        ]
    }

    #[test]
    fn a_full_sentence_surfaces_the_right_commands() {
        let c = corpus();
        let hits = search(
            "i ran a command yesterday that helped me get all my docker container that were active",
            &c,
            10,
        );
        // The sentence names both "all" and "active", so both docker commands are
        // relevant and should top the results; the unrelated git entry should not.
        let top: Vec<i64> = hits.iter().take(2).map(|m| m.id).collect();
        assert!(
            top.contains(&1) && top.contains(&2),
            "expected docker commands on top, got {top:?}"
        );
    }

    #[test]
    fn finds_by_purpose_words_not_in_the_command() {
        let c = corpus();
        let hits = search("docker containers", &c, 10);
        assert!(hits.iter().any(|m| m.id == 1));
        assert!(hits.iter().any(|m| m.id == 2));
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

    #[test]
    fn match_positions_are_sorted_and_in_bounds() {
        let pos = match_positions("dkr", "docker");
        assert!(!pos.is_empty());
        assert!(pos.iter().all(|&i| i < "docker".chars().count()));
        assert!(pos.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn explain_match_splits_matched_from_dropped_filler() {
        let (matched, dropped) =
            explain_match("the docker containers", "docker ps active containers");
        assert!(matched.contains(&"docker".to_string()));
        assert!(matched.contains(&"containers".to_string()));
        assert!(dropped.contains(&"the".to_string()));
    }

    #[test]
    fn frecency_ranks_recent_heavy_use_first_and_sinks_drafts() {
        let mut heavy = mem(1, "docker ps", Some("used a lot"), &[]);
        heavy.use_count = 20;
        heavy.last_used_at = Some(1_000_000);
        let light = mem(2, "git log", Some("rarely"), &[]);
        let draft = mem(3, "rm -rf tmp", None, &[]); // no description = draft
        let memories = vec![light, heavy, draft];
        let order = frecency_order(&memories, 1_000_000);
        assert_eq!(memories[order[0]].id, 1);
        assert_eq!(memories[order[2]].id, 3);
    }

    #[test]
    fn curated_ranks_ahead_of_a_draft_for_the_same_command() {
        let draft = mem(1, "docker ps", None, &["docker"]);
        let curated = mem(2, "docker ps", Some("list running containers"), &["docker"]);
        let memories = [draft, curated];
        assert_eq!(
            search("docker", &memories, 10).first().map(|m| m.id),
            Some(2)
        );
    }

    #[test]
    fn equal_matches_break_toward_the_more_used() {
        let mut a = mem(1, "docker ps", Some("list containers"), &["docker"]);
        let mut b = mem(2, "docker ps", Some("list containers"), &["docker"]);
        a.use_count = 0;
        b.use_count = 12;
        let memories = [a, b];
        let hits = search("docker", &memories, 10);
        assert_eq!(hits.first().map(|m| m.id), Some(2));
    }
}
