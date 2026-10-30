//! SQLite ownership and persistent learning state.
mod dataset;
mod history;
mod labels;
mod schema;
mod vocabulary;

pub use labels::{edits, label};
use schema::migrate;

use crate::config::{err, Result};
use rusqlite::{params, Connection};
#[cfg(test)]
use serde_json::json;
use serde_json::Value;
use std::path::{Path, PathBuf};
pub struct Store {
    pub db: Connection,
    pub root: PathBuf,
    latest_audio: std::cell::RefCell<Option<(i64, Vec<f32>)>>,
}
pub fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
impl Store {
    pub fn open(root: &Path) -> Result<Self> {
        let db = Connection::open(root.join("learning.db")).map_err(err)?;
        db.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(err)?;
        db.execute_batch("PRAGMA journal_mode=WAL;").map_err(err)?;
        migrate(&db)?;
        Ok(Self {
            db,
            root: root.into(),
            latest_audio: std::cell::RefCell::new(None),
        })
    }
    pub fn get_state(&self, k: &str) -> Result<Value> {
        let text: String = self
            .db
            .query_row("SELECT value FROM learn_state WHERE key=?", [k], |r| {
                r.get(0)
            })
            .map_err(err)?;
        serde_json::from_str(&text).map_err(err)
    }
    pub fn state(&self, k: &str, v: &Value) -> Result<()> {
        self.db
            .execute(
                "INSERT INTO learn_state VALUES(?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![k, v.to_string()],
            )
            .map_err(err)?;
        Ok(())
    }
}
#[cfg(test)]
#[path = "../../tests/unit/store_labels.rs"]
mod tests;

#[cfg(test)]
#[path = "../../tests/unit/store_persistence.rs"]
mod persistence_tests;
