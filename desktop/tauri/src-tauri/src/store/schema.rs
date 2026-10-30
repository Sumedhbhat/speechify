//! Create the learning schema and migrate older databases before use.
use crate::config::{err, Result};
use rusqlite::Connection;

pub(super) fn migrate(db: &Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS samples (
            id INTEGER PRIMARY KEY,
            ts REAL NOT NULL,
            audio TEXT,
            raw TEXT NOT NULL,
            pasted TEXT NOT NULL,
            corrected TEXT,
            excluded INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS rules (
            wrong TEXT NOT NULL,
            right TEXT NOT NULL,
            count INTEGER NOT NULL,
            PRIMARY KEY (wrong, right)
        );
        CREATE TABLE IF NOT EXISTS vocab (
            term TEXT PRIMARY KEY,
            count INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS learn_state (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );",
    )
    .map_err(err)?;
    let cols: Vec<String> = db
        .prepare("PRAGMA table_info(samples)")
        .map_err(err)?
        .query_map([], |r| r.get(1))
        .map_err(err)?
        .collect::<std::result::Result<_, _>>()
        .map_err(err)?;
    for (k, t) in [
        ("excluded", "INTEGER NOT NULL DEFAULT 0"),
        ("teacher_text", "TEXT"),
        ("teacher_model", "TEXT"),
        ("teacher_confidence", "REAL"),
        ("teacher_ts", "REAL"),
        ("teacher_note", "TEXT"),
    ] {
        if !cols.iter().any(|s| s == k) {
            db.execute(&format!("ALTER TABLE samples ADD COLUMN {k} {t}"), [])
                .map_err(err)?;
        }
    }
    Ok(())
}
