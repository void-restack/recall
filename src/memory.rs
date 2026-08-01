use std::collections::HashSet;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandMemory {
    pub id: i64,
    pub command: String,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub use_count: i64,
}

impl CommandMemory {
    pub fn is_draft(&self) -> bool {
        self.description.as_deref().is_none_or(str::is_empty)
    }
}

#[derive(Debug, Clone)]
pub struct NewMemory {
    pub command: String,
    pub description: Option<String>,
    pub tags: Vec<String>,
}

/// Normalize so `Docker`, `docker`, and ` docker ` don't fragment the collection.
pub fn normalize_tags(raw: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for tag in raw {
        let norm = tag.trim().to_lowercase().replace(char::is_whitespace, "-");
        if !norm.is_empty() && seen.insert(norm.clone()) {
            out.push(norm);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_lowercases_hyphenates_and_dedupes() {
        let got = normalize_tags(vec![
            "  Docker ".into(),
            "docker".into(),
            "Disk Cleanup".into(),
        ]);
        assert_eq!(got, vec!["docker".to_string(), "disk-cleanup".to_string()]);
    }

    #[test]
    fn missing_or_empty_description_is_a_draft() {
        let mut m = CommandMemory {
            id: 1,
            command: "docker system prune".into(),
            description: None,
            tags: vec![],
            created_at: 0,
            updated_at: 0,
            use_count: 0,
        };
        assert!(m.is_draft());
        m.description = Some("reclaim disk".into());
        assert!(!m.is_draft());
    }
}
