use std::path::PathBuf;

use anyhow::{Context, Result};
use directories::ProjectDirs;

pub fn database_path() -> Result<PathBuf> {
    let dirs = ProjectDirs::from("", "", "recall")
        .context("could not resolve a platform data directory")?;
    let data_dir = dirs.data_dir();
    std::fs::create_dir_all(data_dir)
        .with_context(|| format!("creating data directory {}", data_dir.display()))?;
    Ok(data_dir.join("recall.db"))
}
