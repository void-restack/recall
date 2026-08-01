use std::path::PathBuf;
use std::sync::OnceLock;

use anyhow::{Context, Result};
use directories::ProjectDirs;

static DB_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();

/// Record a database path chosen via `--db` / `RECALL_DB`. Set once at startup.
pub fn set_db_override(path: Option<PathBuf>) {
    if let Some(path) = path {
        let _ = DB_OVERRIDE.set(path);
    }
}

fn project_dirs() -> Result<ProjectDirs> {
    ProjectDirs::from("", "", "recall").context("could not resolve a platform data directory")
}

pub fn database_path() -> Result<PathBuf> {
    if let Some(path) = DB_OVERRIDE.get() {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        return Ok(path.clone());
    }

    let dirs = project_dirs()?;
    let data_dir = dirs.data_dir();
    std::fs::create_dir_all(data_dir)
        .with_context(|| format!("creating data directory {}", data_dir.display()))?;
    Ok(data_dir.join("recall.db"))
}

/// Where the shell hook records the previous command for `add --last` to read.
pub fn last_command_path() -> Result<PathBuf> {
    Ok(project_dirs()?.cache_dir().join("last-command"))
}
