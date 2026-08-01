use std::path::PathBuf;

use anyhow::{Context, Result};
use directories::ProjectDirs;

fn project_dirs() -> Result<ProjectDirs> {
    ProjectDirs::from("", "", "recall").context("could not resolve a platform data directory")
}

pub fn database_path() -> Result<PathBuf> {
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
