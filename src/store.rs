use anyhow::{Context, Result};
use rusqlite::{Connection, Row};

use crate::memory::{CommandMemory, NewMemory};
use crate::paths;

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open() -> Result<Self> {
        let path = paths::database_path()?;
        let conn = Connection::open(&path)
            .with_context(|| format!("opening database {}", path.display()))?;
        // WAL lets readers run while one process writes; busy_timeout waits out a concurrent writer.
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;

        #[cfg(unix)]
        restrict_permissions(&path)?;

        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<()> {
        // Each step runs once and bumps `user_version`; new steps append below.
        let version: i64 = self.conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version < 1 {
            self.conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS memories (
                    id          INTEGER PRIMARY KEY AUTOINCREMENT,
                    command     TEXT    NOT NULL,
                    description TEXT,
                    tags        TEXT    NOT NULL DEFAULT '[]',
                    created_at  INTEGER NOT NULL,
                    updated_at  INTEGER NOT NULL,
                    use_count   INTEGER NOT NULL DEFAULT 0
                );
                PRAGMA user_version = 1;",
            )?;
        }
        Ok(())
    }

    pub fn insert(&self, new: &NewMemory, now: i64) -> Result<CommandMemory> {
        let tags_json = serde_json::to_string(&new.tags)?;
        self.conn.execute(
            "INSERT INTO memories (command, description, tags, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4)",
            rusqlite::params![new.command, new.description, tags_json, now],
        )?;
        Ok(CommandMemory {
            id: self.conn.last_insert_rowid(),
            command: new.command.clone(),
            description: new.description.clone(),
            tags: new.tags.clone(),
            created_at: now,
            updated_at: now,
            use_count: 0,
        })
    }

    pub fn get(&self, id: i64) -> Result<Option<CommandMemory>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, command, description, tags, created_at, updated_at, use_count
             FROM memories WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map([id], row_to_memory)?;
        rows.next().transpose().context("reading memory")
    }

    pub fn list(&self) -> Result<Vec<CommandMemory>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, command, description, tags, created_at, updated_at, use_count
             FROM memories ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], row_to_memory)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("reading memories")
    }

    pub fn record_use(&self, id: i64) -> Result<()> {
        self.conn
            .execute("UPDATE memories SET use_count = use_count + 1 WHERE id = ?1", [id])?;
        Ok(())
    }
}

fn row_to_memory(row: &Row) -> rusqlite::Result<CommandMemory> {
    let tags_json: String = row.get("tags")?;
    Ok(CommandMemory {
        id: row.get("id")?,
        command: row.get("command")?,
        description: row.get("description")?,
        tags: serde_json::from_str(&tags_json).unwrap_or_default(),
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        use_count: row.get("use_count")?,
    })
}

#[cfg(unix)]
fn restrict_permissions(path: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("restricting permissions on {}", path.display()))
}
