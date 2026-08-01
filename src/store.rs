use anyhow::{Context, Result};
use rusqlite::{Connection, Row};

use crate::memory::{CommandMemory, ImportRecord, NewMemory};
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

    #[cfg(test)]
    fn in_memory() -> Result<Self> {
        let store = Self { conn: Connection::open_in_memory()? };
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
        if version < 2 {
            self.conn.execute_batch(
                "ALTER TABLE memories ADD COLUMN last_used_at INTEGER;
                 PRAGMA user_version = 2;",
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
            last_used_at: None,
        })
    }

    pub fn get(&self, id: i64) -> Result<Option<CommandMemory>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, command, description, tags, created_at, updated_at, use_count, last_used_at
             FROM memories WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map([id], row_to_memory)?;
        rows.next().transpose().context("reading memory")
    }

    pub fn list(&self) -> Result<Vec<CommandMemory>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, command, description, tags, created_at, updated_at, use_count, last_used_at
             FROM memories ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], row_to_memory)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("reading memories")
    }

    pub fn update(&self, m: &CommandMemory, now: i64) -> Result<()> {
        let tags_json = serde_json::to_string(&m.tags)?;
        let changed = self.conn.execute(
            "UPDATE memories SET command = ?2, description = ?3, tags = ?4, updated_at = ?5
             WHERE id = ?1",
            rusqlite::params![m.id, m.command, m.description, tags_json, now],
        )?;
        if changed == 0 {
            anyhow::bail!("no memory #{}", m.id);
        }
        Ok(())
    }

    pub fn delete(&self, id: i64) -> Result<bool> {
        let changed = self
            .conn
            .execute("DELETE FROM memories WHERE id = ?1", [id])?;
        Ok(changed > 0)
    }

    pub fn record_use(&self, id: i64, now: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE memories SET use_count = use_count + 1, last_used_at = ?2 WHERE id = ?1",
            rusqlite::params![id, now],
        )?;
        Ok(())
    }

    pub fn ids_with_command(&self, command: &str) -> Result<Vec<i64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM memories WHERE command = ?1 ORDER BY id")?;
        stmt.query_map([command], |row| row.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("looking for duplicates")
    }

    /// Insert imported records additively (fresh ids, preserved timestamps/usage)
    /// in one transaction, so a mid-import failure leaves the store untouched.
    pub fn import_all(&self, records: &[ImportRecord], now: i64) -> Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        for record in records {
            let created = record.created_at.unwrap_or(now);
            let updated = record.updated_at.unwrap_or(created);
            let tags = crate::memory::normalize_tags(record.tags.clone());
            let tags_json = serde_json::to_string(&tags)?;
            tx.execute(
                "INSERT INTO memories
                 (command, description, tags, created_at, updated_at, use_count, last_used_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    record.command,
                    record.description,
                    tags_json,
                    created,
                    updated,
                    record.use_count,
                    record.last_used_at,
                ],
            )?;
        }
        tx.commit()?;
        Ok(records.len())
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
        last_used_at: row.get("last_used_at")?,
    })
}

#[cfg(unix)]
fn restrict_permissions(path: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("restricting permissions on {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> NewMemory {
        NewMemory {
            command: "docker ps".into(),
            description: Some("list containers".into()),
            tags: vec!["docker".into()],
        }
    }

    #[test]
    fn insert_get_update_delete_roundtrip() {
        let store = Store::in_memory().unwrap();

        let saved = store.insert(&sample(), 100).unwrap();
        assert_eq!(saved.id, 1);
        assert_eq!(store.get(1).unwrap().unwrap().command, "docker ps");

        let mut edited = saved.clone();
        edited.description = Some("list running containers".into());
        store.update(&edited, 200).unwrap();
        let reloaded = store.get(1).unwrap().unwrap();
        assert_eq!(reloaded.description.as_deref(), Some("list running containers"));
        assert_eq!(reloaded.updated_at, 200);

        assert!(store.delete(1).unwrap());
        assert!(store.get(1).unwrap().is_none());
        assert!(!store.delete(1).unwrap());
    }

    #[test]
    fn record_use_counts_and_stamps_last_used() {
        let store = Store::in_memory().unwrap();
        let m = store.insert(&sample(), 0).unwrap();
        assert!(m.last_used_at.is_none());
        store.record_use(m.id, 300).unwrap();
        store.record_use(m.id, 500).unwrap();
        let reloaded = store.get(m.id).unwrap().unwrap();
        assert_eq!(reloaded.use_count, 2);
        assert_eq!(reloaded.last_used_at, Some(500));
    }

    #[test]
    fn ids_with_command_finds_duplicates() {
        let store = Store::in_memory().unwrap();
        store.insert(&sample(), 1).unwrap();
        store.insert(&sample(), 2).unwrap();
        assert_eq!(store.ids_with_command("docker ps").unwrap(), vec![1, 2]);
        assert!(store.ids_with_command("nope").unwrap().is_empty());
    }

    #[test]
    fn import_all_adds_records_with_fresh_ids() {
        let store = Store::in_memory().unwrap();
        let records = vec![ImportRecord {
            command: "docker ps".into(),
            description: Some("list".into()),
            tags: vec!["docker".into()],
            created_at: Some(10),
            updated_at: Some(10),
            use_count: 5,
            last_used_at: Some(20),
        }];
        assert_eq!(store.import_all(&records, 999).unwrap(), 1);
        let all = store.list().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, 1);
        assert_eq!(all[0].command, "docker ps");
        assert_eq!(all[0].use_count, 5);
        assert_eq!(all[0].created_at, 10);
    }
}
