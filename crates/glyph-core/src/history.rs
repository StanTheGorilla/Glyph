//! Local SQLite history of past dictations.

use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::Serialize;

#[derive(Serialize, Clone, Debug)]
pub struct Entry {
    pub id: i64,
    pub ts: i64, // unix seconds
    pub app: String,
    pub raw: String,
    pub clean: String,
}

/// A history store. Multiple connections to the same file are fine (WAL mode):
/// the engine writes, the UI reads.
pub struct History {
    conn: Mutex<Connection>,
}

impl History {
    pub fn open(path: &Path) -> Result<History> {
        let conn = Connection::open(path).with_context(|| format!("open {}", path.display()))?;
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        let _ = conn.busy_timeout(std::time::Duration::from_secs(3));
        conn.execute(
            "CREATE TABLE IF NOT EXISTS dictations (
                id    INTEGER PRIMARY KEY AUTOINCREMENT,
                ts    INTEGER NOT NULL,
                app   TEXT NOT NULL,
                raw   TEXT NOT NULL,
                clean TEXT NOT NULL
            )",
            [],
        )?;
        Ok(History { conn: Mutex::new(conn) })
    }

    pub fn record(&self, raw: &str, clean: &str, app: &str) {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute(
                "INSERT INTO dictations (ts, app, raw, clean) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![ts, app, raw, clean],
            );
        }
    }

    pub fn recent(&self, limit: usize) -> Result<Vec<Entry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id, ts, app, raw, clean FROM dictations ORDER BY id DESC LIMIT ?1")?;
        let rows = stmt.query_map([limit as i64], |r| {
            Ok(Entry {
                id: r.get(0)?,
                ts: r.get(1)?,
                app: r.get(2)?,
                raw: r.get(3)?,
                clean: r.get(4)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn clear(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM dictations", [])?;
        Ok(())
    }
}
