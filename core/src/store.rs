//! Everything the app remembers, in one SQLite file.

use crate::context::{Category, Rule};
use crate::format::Replacement;
use crate::settings::{Retention, Settings};
use crate::{err, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use std::path::Path;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS kv (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS dictations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at INTEGER NOT NULL,
    offset_min INTEGER NOT NULL,
    app TEXT NOT NULL, process TEXT NOT NULL, title TEXT NOT NULL, category TEXT NOT NULL,
    command INTEGER NOT NULL,
    duration_ms INTEGER NOT NULL, speech_ms INTEGER NOT NULL,
    raw TEXT NOT NULL, text TEXT NOT NULL, words INTEGER NOT NULL,
    rewritten INTEGER NOT NULL, speech_model TEXT NOT NULL, rewrite_model TEXT NOT NULL,
    asr_ms INTEGER NOT NULL, rewrite_ms INTEGER NOT NULL, total_ms INTEGER NOT NULL,
    outcome TEXT NOT NULL, fixes INTEGER NOT NULL, audio TEXT NOT NULL,
    edited TEXT, teacher_raw TEXT, teacher_text TEXT, teacher_wer REAL, reviewed_at INTEGER,
    flagged INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS dictations_time ON dictations (created_at);
CREATE TABLE IF NOT EXISTS dictionary (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    heard TEXT NOT NULL, term TEXT NOT NULL, source TEXT NOT NULL,
    hits INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL,
    UNIQUE (heard, term)
);
CREATE TABLE IF NOT EXISTS forgotten (key TEXT PRIMARY KEY);
CREATE TABLE IF NOT EXISTS snippets (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    trigger TEXT NOT NULL UNIQUE, expansion TEXT NOT NULL, hits INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS notes (
    id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL, body TEXT NOT NULL, updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS suggestions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    heard TEXT NOT NULL, term TEXT NOT NULL, count INTEGER NOT NULL, status TEXT NOT NULL DEFAULT 'open',
    UNIQUE (heard, term)
);
CREATE TABLE IF NOT EXISTS examples (
    id INTEGER PRIMARY KEY AUTOINCREMENT, category TEXT NOT NULL, raw TEXT NOT NULL, gold TEXT NOT NULL, created_at INTEGER NOT NULL
);
";

/// Wall-clock time supplied by the shell so the core stays deterministic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clock {
    /// Seconds since the Unix epoch.
    pub now: i64,
    /// Local offset from UTC in minutes (India is 330).
    pub offset_min: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    #[default]
    Pasted,
    /// No text field was focused; the text was offered for copying.
    Copied,
    Cancelled,
    Failed,
}

impl Outcome {
    fn name(self) -> &'static str {
        match self {
            Outcome::Pasted => "pasted",
            Outcome::Copied => "copied",
            Outcome::Cancelled => "cancelled",
            Outcome::Failed => "failed",
        }
    }

    fn parse(name: &str) -> Outcome {
        match name {
            "copied" => Outcome::Copied,
            "cancelled" => Outcome::Cancelled,
            "failed" => Outcome::Failed,
            _ => Outcome::Pasted,
        }
    }
}

fn category_name(category: Category) -> String {
    serde_json::to_value(category)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn category_parse(name: &str) -> Category {
    serde_json::from_value(serde_json::Value::String(name.into())).unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Dictation {
    pub id: i64,
    pub created_at: i64,
    pub offset_min: i32,
    pub app: String,
    pub process: String,
    pub title: String,
    pub category: Category,
    pub command: bool,
    /// Length of the recording.
    pub duration_ms: u32,
    /// Time actually spent speaking.
    pub speech_ms: u32,
    /// What the recogniser heard.
    pub raw: String,
    /// What was pasted.
    pub text: String,
    pub words: u32,
    pub rewritten: bool,
    pub speech_model: String,
    pub rewrite_model: String,
    pub asr_ms: u32,
    pub rewrite_ms: u32,
    pub total_ms: u32,
    pub outcome: Outcome,
    pub fixes: u32,
    /// File name of the recording inside the audio folder; empty if not kept.
    pub audio: String,
    /// The user's corrected version, if they fixed it in History.
    pub edited: Option<String>,
    pub teacher_raw: Option<String>,
    pub teacher_text: Option<String>,
    pub teacher_wer: Option<f64>,
    pub reviewed_at: Option<i64>,
    pub flagged: bool,
}

const DICTATION_COLUMNS: &str = "id, created_at, offset_min, app, process, title, category, command, duration_ms, speech_ms, raw, text, words, rewritten, speech_model, rewrite_model, asr_ms, rewrite_ms, total_ms, outcome, fixes, audio, edited, teacher_raw, teacher_text, teacher_wer, reviewed_at, flagged";

fn read_dictation(row: &Row) -> rusqlite::Result<Dictation> {
    Ok(Dictation {
        id: row.get(0)?,
        created_at: row.get(1)?,
        offset_min: row.get(2)?,
        app: row.get(3)?,
        process: row.get(4)?,
        title: row.get(5)?,
        category: category_parse(&row.get::<_, String>(6)?),
        command: row.get(7)?,
        duration_ms: row.get(8)?,
        speech_ms: row.get(9)?,
        raw: row.get(10)?,
        text: row.get(11)?,
        words: row.get(12)?,
        rewritten: row.get(13)?,
        speech_model: row.get(14)?,
        rewrite_model: row.get(15)?,
        asr_ms: row.get(16)?,
        rewrite_ms: row.get(17)?,
        total_ms: row.get(18)?,
        outcome: Outcome::parse(&row.get::<_, String>(19)?),
        fixes: row.get(20)?,
        audio: row.get(21)?,
        edited: row.get(22)?,
        teacher_raw: row.get(23)?,
        teacher_text: row.get(24)?,
        teacher_wer: row.get(25)?,
        reviewed_at: row.get(26)?,
        flagged: row.get(27)?,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Word {
    pub id: i64,
    /// What the recogniser writes instead; empty for a plain vocabulary word.
    pub heard: String,
    /// The correct spelling.
    pub term: String,
    /// "manual", "edit" (learned from a correction) or "teacher".
    pub source: String,
    pub hits: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snippet {
    pub id: i64,
    pub trigger: String,
    pub expansion: String,
    pub hits: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    pub id: i64,
    pub title: String,
    pub body: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Suggestion {
    pub id: i64,
    pub heard: String,
    pub term: String,
    pub count: u32,
}

/// The replacements the formatter applies, with where each came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rules {
    pub replacements: Vec<Replacement>,
    /// Parallel to `replacements`: (is a snippet, row id).
    origins: Vec<(bool, i64)>,
}

pub const MAX_TERM: usize = 60;
pub const MAX_EXPANSION: usize = 4000;

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        Self::init(Connection::open(path).map_err(err)?)
    }

    pub fn memory() -> Store {
        Self::init(Connection::open_in_memory().expect("SQLite can always open memory")).expect("the schema is valid")
    }

    fn init(conn: Connection) -> Result<Store> {
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")
            .map_err(err)?;
        conn.execute_batch(SCHEMA).map_err(err)?;
        Ok(Store { conn })
    }

    // ---- key/value and settings ----

    pub fn get(&self, key: &str) -> Option<String> {
        self.conn
            .query_row("SELECT value FROM kv WHERE key = ?1", [key], |row| row.get(0))
            .optional()
            .ok()
            .flatten()
    }

    pub fn set(&self, key: &str, value: &str) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO kv (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = ?2",
                params![key, value],
            )
            .map_err(err)?;
        Ok(())
    }

    /// Unreadable or missing settings fall back to defaults rather than
    /// stopping the app from starting.
    pub fn settings(&self) -> Settings {
        self.get("settings")
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default()
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        self.set("settings", &serde_json::to_string(settings).map_err(err)?)
    }

    pub fn rules(&self) -> Vec<Rule> {
        self.get("rules")
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default()
    }

    pub fn save_rules(&self, rules: &[Rule]) -> Result<()> {
        self.set("rules", &serde_json::to_string(rules).map_err(err)?)
    }

    // ---- dictations ----

    pub fn add_dictation(&self, d: &Dictation) -> Result<i64> {
        self.conn
            .execute(
                "INSERT INTO dictations (created_at, offset_min, app, process, title, category, command, duration_ms, speech_ms, raw, text, words, rewritten, speech_model, rewrite_model, asr_ms, rewrite_ms, total_ms, outcome, fixes, audio)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)",
                params![d.created_at, d.offset_min, d.app, d.process, d.title, category_name(d.category), d.command, d.duration_ms, d.speech_ms, d.raw, d.text, d.words, d.rewritten, d.speech_model, d.rewrite_model, d.asr_ms, d.rewrite_ms, d.total_ms, d.outcome.name(), d.fixes, d.audio],
            )
            .map_err(err)?;
        Ok(self.conn.last_insert_rowid())
    }

    fn select(&self, tail: &str, args: &[&dyn rusqlite::ToSql]) -> Result<Vec<Dictation>> {
        let mut statement = self
            .conn
            .prepare(&format!("SELECT {DICTATION_COLUMNS} FROM dictations {tail}"))
            .map_err(err)?;
        let rows = statement.query_map(args, read_dictation).map_err(err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(err)
    }

    /// Newest first. `search` matches the text, what was heard and the app.
    pub fn history(&self, search: &str, limit: u32, offset: u32) -> Result<Vec<Dictation>> {
        let like = format!("%{}%", search.trim().replace('%', "\\%").replace('_', "\\_"));
        self.select(
            "WHERE text LIKE ?1 ESCAPE '\\' OR raw LIKE ?1 ESCAPE '\\' OR app LIKE ?1 ESCAPE '\\' ORDER BY id DESC LIMIT ?2 OFFSET ?3",
            params![like, limit, offset],
        )
    }

    pub fn dictation(&self, id: i64) -> Result<Dictation> {
        self.select("WHERE id = ?1", params![id])?
            .pop()
            .ok_or_else(|| "That dictation no longer exists".to_string())
    }

    /// The most recent dictation that produced text, for "paste last".
    pub fn last(&self) -> Option<Dictation> {
        self.select("WHERE text != '' ORDER BY id DESC LIMIT 1", params![])
            .ok()?
            .pop()
    }

    /// Every dictation, oldest first, for insights.
    pub fn all(&self) -> Result<Vec<Dictation>> {
        self.select("ORDER BY id", params![])
    }

    pub fn set_outcome(&self, id: i64, outcome: Outcome) -> Result<()> {
        self.conn
            .execute(
                "UPDATE dictations SET outcome = ?2 WHERE id = ?1",
                params![id, outcome.name()],
            )
            .map_err(err)?;
        Ok(())
    }

    /// Records the user's corrected text. An empty or unchanged edit clears it.
    pub fn set_edited(&self, id: i64, text: &str) -> Result<()> {
        let current = self.dictation(id)?;
        let edited = Some(text.trim()).filter(|t| !t.is_empty() && *t != current.text);
        self.conn
            .execute("UPDATE dictations SET edited = ?2 WHERE id = ?1", params![id, edited])
            .map_err(err)?;
        Ok(())
    }

    pub fn set_flagged(&self, id: i64, flagged: bool) -> Result<()> {
        self.conn
            .execute("UPDATE dictations SET flagged = ?2 WHERE id = ?1", params![id, flagged])
            .map_err(err)?;
        Ok(())
    }

    /// Deletes dictations and returns their audio files for removal.
    pub fn delete_dictations(&self, ids: Option<&[i64]>) -> Result<Vec<String>> {
        let rows = match ids {
            Some(ids) => ids.iter().filter_map(|id| self.dictation(*id).ok()).collect(),
            None => self.all()?,
        };
        for row in &rows {
            self.conn
                .execute("DELETE FROM dictations WHERE id = ?1", [row.id])
                .map_err(err)?;
        }
        Ok(rows
            .into_iter()
            .map(|row| row.audio)
            .filter(|audio| !audio.is_empty())
            .collect())
    }

    /// Applies the retention setting; returns audio files to remove.
    pub fn prune(&self, retention: Retention, clock: Clock) -> Result<Vec<String>> {
        let cutoff = match retention {
            Retention::Keep => return Ok(vec![]),
            Retention::Day => clock.now - 86_400,
            Retention::Never => i64::MAX,
        };
        let old: Vec<i64> = self
            .select("WHERE created_at < ?1", params![cutoff])?
            .iter()
            .map(|d| d.id)
            .collect();
        self.delete_dictations(Some(&old))
    }

    // ---- dictionary ----

    pub fn dictionary(&self) -> Result<Vec<Word>> {
        let mut statement = self
            .conn
            .prepare("SELECT id, heard, term, source, hits FROM dictionary ORDER BY term COLLATE NOCASE, heard")
            .map_err(err)?;
        let rows = statement
            .query_map([], |row| {
                Ok(Word {
                    id: row.get(0)?,
                    heard: row.get(1)?,
                    term: row.get(2)?,
                    source: row.get(3)?,
                    hits: row.get(4)?,
                })
            })
            .map_err(err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(err)
    }

    fn forgotten_key(heard: &str, term: &str) -> String {
        format!("{}\u{1}{}", heard.trim().to_lowercase(), term.trim().to_lowercase())
    }

    pub fn is_forgotten(&self, heard: &str, term: &str) -> bool {
        self.conn
            .query_row(
                "SELECT 1 FROM forgotten WHERE key = ?1",
                [Self::forgotten_key(heard, term)],
                |_| Ok(()),
            )
            .optional()
            .ok()
            .flatten()
            .is_some()
    }

    /// Adds a word, or a correction when `heard` is given. Learned entries
    /// (`source` other than "manual") are skipped if the user deleted them before.
    pub fn add_word(&self, heard: &str, term: &str, source: &str, clock: Clock) -> Result<bool> {
        let (heard, term) = (heard.trim().to_lowercase(), term.trim());
        if term.is_empty() {
            return Err("Enter a word".into());
        }
        if term.chars().count() > MAX_TERM || heard.chars().count() > MAX_TERM {
            return Err(format!("Dictionary entries can be at most {MAX_TERM} characters"));
        }
        if heard == term.to_lowercase() && heard == term {
            return Err("The correction is the same as the original".into());
        }
        if source == "manual" {
            self.conn
                .execute(
                    "DELETE FROM forgotten WHERE key = ?1",
                    [Self::forgotten_key(&heard, term)],
                )
                .map_err(err)?;
        } else if self.is_forgotten(&heard, term) {
            return Ok(false);
        }
        let added = self
            .conn
            .execute(
                "INSERT OR IGNORE INTO dictionary (heard, term, source, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![heard, term, source, clock.now],
            )
            .map_err(err)?;
        Ok(added > 0)
    }

    /// Deletes a word and remembers not to learn it again.
    pub fn remove_word(&self, id: i64) -> Result<()> {
        let found: Option<(String, String)> = self
            .conn
            .query_row("SELECT heard, term FROM dictionary WHERE id = ?1", [id], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .optional()
            .map_err(err)?;
        if let Some((heard, term)) = found {
            self.conn
                .execute(
                    "INSERT OR IGNORE INTO forgotten (key) VALUES (?1)",
                    [Self::forgotten_key(&heard, &term)],
                )
                .map_err(err)?;
            self.conn
                .execute("DELETE FROM dictionary WHERE id = ?1", [id])
                .map_err(err)?;
        }
        Ok(())
    }

    /// Terms given to the recogniser and the language model as known spellings.
    pub fn vocabulary(&self) -> Vec<String> {
        let mut terms: Vec<String> = Vec::new();
        for word in self.dictionary().unwrap_or_default() {
            if !terms.contains(&word.term) {
                terms.push(word.term);
            }
        }
        terms
    }

    // ---- snippets ----

    pub fn snippets(&self) -> Result<Vec<Snippet>> {
        let mut statement = self
            .conn
            .prepare("SELECT id, trigger, expansion, hits FROM snippets ORDER BY trigger COLLATE NOCASE")
            .map_err(err)?;
        let rows = statement
            .query_map([], |row| {
                Ok(Snippet {
                    id: row.get(0)?,
                    trigger: row.get(1)?,
                    expansion: row.get(2)?,
                    hits: row.get(3)?,
                })
            })
            .map_err(err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(err)
    }

    /// Creates a snippet, or updates the one with `id`.
    pub fn save_snippet(&self, id: Option<i64>, trigger: &str, expansion: &str) -> Result<i64> {
        let trigger = trigger.trim();
        if trigger.is_empty() || expansion.trim().is_empty() {
            return Err("A snippet needs both a phrase and its text".into());
        }
        if trigger.chars().count() > MAX_TERM {
            return Err(format!("The phrase can be at most {MAX_TERM} characters"));
        }
        if expansion.chars().count() > MAX_EXPANSION {
            return Err(format!("Snippet text can be at most {MAX_EXPANSION} characters"));
        }
        let taken: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM snippets WHERE trigger = ?1 COLLATE NOCASE",
                [trigger],
                |row| row.get(0),
            )
            .optional()
            .map_err(err)?;
        if taken.is_some_and(|other| Some(other) != id) {
            return Err(format!("There is already a snippet for \"{trigger}\""));
        }
        match id {
            Some(id) => {
                self.conn
                    .execute(
                        "UPDATE snippets SET trigger = ?2, expansion = ?3 WHERE id = ?1",
                        params![id, trigger, expansion],
                    )
                    .map_err(err)?;
                Ok(id)
            }
            None => {
                self.conn
                    .execute(
                        "INSERT INTO snippets (trigger, expansion) VALUES (?1, ?2)",
                        params![trigger, expansion],
                    )
                    .map_err(err)?;
                Ok(self.conn.last_insert_rowid())
            }
        }
    }

    pub fn delete_snippet(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM snippets WHERE id = ?1", [id])
            .map_err(err)?;
        Ok(())
    }

    // ---- formatter rules ----

    /// Dictionary entries and snippets as replacements for the formatter.
    pub fn replacement_rules(&self) -> Rules {
        let mut rules = Rules {
            replacements: vec![],
            origins: vec![],
        };
        for word in self.dictionary().unwrap_or_default() {
            // A plain word still fixes its own capitalisation: "github" → "GitHub".
            let from = if word.heard.is_empty() { word.term.clone() } else { word.heard };
            rules.replacements.push(Replacement { from, to: word.term });
            rules.origins.push((false, word.id));
        }
        for snippet in self.snippets().unwrap_or_default() {
            rules.replacements.push(Replacement {
                from: snippet.trigger,
                to: snippet.expansion,
            });
            rules.origins.push((true, snippet.id));
        }
        rules
    }

    /// Counts a use of each rule that fired (indexes into `rules.replacements`).
    pub fn record_hits(&self, rules: &Rules, fired: &[usize]) -> Result<()> {
        for index in fired {
            if let Some((snippet, id)) = rules.origins.get(*index) {
                let table = if *snippet { "snippets" } else { "dictionary" };
                self.conn
                    .execute(&format!("UPDATE {table} SET hits = hits + 1 WHERE id = ?1"), [id])
                    .map_err(err)?;
            }
        }
        Ok(())
    }

    // ---- notes ----

    pub fn notes(&self) -> Result<Vec<Note>> {
        let mut statement = self
            .conn
            .prepare("SELECT id, title, body, updated_at FROM notes ORDER BY updated_at DESC, id DESC")
            .map_err(err)?;
        let rows = statement
            .query_map([], |row| {
                Ok(Note {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    body: row.get(2)?,
                    updated_at: row.get(3)?,
                })
            })
            .map_err(err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(err)
    }

    pub fn save_note(&self, id: Option<i64>, title: &str, body: &str, clock: Clock) -> Result<i64> {
        match id {
            Some(id) => {
                self.conn
                    .execute(
                        "UPDATE notes SET title = ?2, body = ?3, updated_at = ?4 WHERE id = ?1",
                        params![id, title, body, clock.now],
                    )
                    .map_err(err)?;
                Ok(id)
            }
            None => {
                self.conn
                    .execute(
                        "INSERT INTO notes (title, body, updated_at) VALUES (?1, ?2, ?3)",
                        params![title, body, clock.now],
                    )
                    .map_err(err)?;
                Ok(self.conn.last_insert_rowid())
            }
        }
    }

    pub fn delete_note(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM notes WHERE id = ?1", [id])
            .map_err(err)?;
        Ok(())
    }

    // ---- learning ----

    /// Counts one more sighting of a correction and returns the new total.
    /// Already-known and forgotten corrections return 0.
    pub fn suggest(&self, heard: &str, term: &str, weight: u32) -> Result<u32> {
        let heard = heard.trim().to_lowercase();
        let known = self
            .dictionary()?
            .iter()
            .any(|word| word.heard == heard && word.term == term);
        if known || self.is_forgotten(&heard, term) {
            return Ok(0);
        }
        self.conn
            .execute(
                "INSERT INTO suggestions (heard, term, count) VALUES (?1, ?2, ?3) ON CONFLICT (heard, term) DO UPDATE SET count = count + ?3",
                params![heard, term, weight],
            )
            .map_err(err)?;
        self.conn
            .query_row(
                "SELECT count FROM suggestions WHERE heard = ?1 AND term = ?2 AND status = 'open'",
                params![heard, term],
                |row| row.get(0),
            )
            .optional()
            .map_err(err)
            .map(|count| count.unwrap_or(0))
    }

    /// Moves a counted correction into the dictionary.
    pub fn accept(&self, heard: &str, term: &str, source: &str, clock: Clock) -> Result<()> {
        self.add_word(heard, term, source, clock)?;
        let sql = "UPDATE suggestions SET status = 'accepted' WHERE heard = ?1 AND term = ?2";
        self.conn.execute(sql, params![heard, term]).map_err(err)?;
        Ok(())
    }

    /// Corrections seen but not yet accepted, most frequent first.
    pub fn suggestions(&self) -> Result<Vec<Suggestion>> {
        let mut statement = self
            .conn
            .prepare("SELECT id, heard, term, count FROM suggestions WHERE status = 'open' ORDER BY count DESC, id")
            .map_err(err)?;
        let rows = statement
            .query_map([], |row| {
                Ok(Suggestion {
                    id: row.get(0)?,
                    heard: row.get(1)?,
                    term: row.get(2)?,
                    count: row.get(3)?,
                })
            })
            .map_err(err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(err)
    }

    /// Accepting adds the correction to the dictionary; rejecting forgets it for good.
    pub fn resolve_suggestion(&self, id: i64, accept: bool, source: &str, clock: Clock) -> Result<()> {
        let found: Option<(String, String)> = self
            .conn
            .query_row("SELECT heard, term FROM suggestions WHERE id = ?1", [id], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .optional()
            .map_err(err)?;
        let Some((heard, term)) = found else {
            return Ok(());
        };
        if accept {
            self.add_word(&heard, &term, source, clock)?;
        } else {
            self.conn
                .execute(
                    "INSERT OR IGNORE INTO forgotten (key) VALUES (?1)",
                    [Self::forgotten_key(&heard, &term)],
                )
                .map_err(err)?;
        }
        self.conn
            .execute(
                "UPDATE suggestions SET status = ?2 WHERE id = ?1",
                params![id, if accept { "accepted" } else { "rejected" }],
            )
            .map_err(err)?;
        Ok(())
    }

    /// Dictations with a recording that the teacher has not reviewed yet.
    pub fn pending_review(&self, limit: usize) -> Result<Vec<Dictation>> {
        self.select(
            "WHERE reviewed_at IS NULL AND audio != '' AND outcome IN ('pasted', 'copied') ORDER BY id LIMIT ?1",
            params![limit as i64],
        )
    }

    pub fn save_review(&self, id: i64, teacher_raw: &str, teacher_text: &str, wer: f64, clock: Clock) -> Result<()> {
        self.conn
            .execute("UPDATE dictations SET teacher_raw = ?2, teacher_text = ?3, teacher_wer = ?4, reviewed_at = ?5 WHERE id = ?1", params![id, teacher_raw, teacher_text, wer, clock.now])
            .map_err(err)?;
        Ok(())
    }

    /// Marks a dictation reviewed without a result (its recording is unreadable).
    pub fn skip_review(&self, id: i64, clock: Clock) -> Result<()> {
        self.conn
            .execute(
                "UPDATE dictations SET reviewed_at = ?2 WHERE id = ?1",
                params![id, clock.now],
            )
            .map_err(err)?;
        Ok(())
    }

    /// Keeps a worked example (what was heard → the ideal text) for the
    /// everyday model's prompt. Only the newest 200 are kept.
    pub fn add_example(&self, category: Category, raw: &str, gold: &str, clock: Clock) -> Result<()> {
        self.conn
            .execute("DELETE FROM examples WHERE raw = ?1", [raw])
            .map_err(err)?;
        self.conn
            .execute(
                "INSERT INTO examples (category, raw, gold, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![category_name(category), raw, gold, clock.now],
            )
            .map_err(err)?;
        self.conn
            .execute(
                "DELETE FROM examples WHERE id NOT IN (SELECT id FROM examples ORDER BY id DESC LIMIT 200)",
                [],
            )
            .map_err(err)?;
        Ok(())
    }

    /// The newest examples, preferring the same kind of app.
    pub fn examples(&self, category: Category, limit: usize) -> Vec<(String, String)> {
        let query = "SELECT raw, gold FROM examples ORDER BY category = ?1 DESC, id DESC LIMIT ?2";
        let read = || -> rusqlite::Result<Vec<(String, String)>> {
            let mut statement = self.conn.prepare(query)?;
            let rows = statement.query_map(params![category_name(category), limit as i64], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?;
            rows.collect()
        };
        read().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::dictation;

    const NOW: Clock = Clock {
        now: 1_800_000_000,
        offset_min: 330,
    };

    #[test]
    fn settings_and_rules_round_trip_and_survive_corruption() {
        let store = Store::memory();
        assert_eq!(store.settings(), Settings::default());
        let mut settings = Settings::default();
        settings.rewrite.model = "qwen3:4b".into();
        store.save_settings(&settings).unwrap();
        assert_eq!(store.settings(), settings);
        store.set("settings", "{broken").unwrap();
        assert_eq!(store.settings(), Settings::default());
        assert!(store.rules().is_empty());
        let rules = vec![Rule {
            pattern: "figma".into(),
            category: Category::Work,
        }];
        store.save_rules(&rules).unwrap();
        assert_eq!(store.rules(), rules);
        assert_eq!(store.get("missing"), None);
    }

    #[test]
    fn opens_a_file_and_reports_a_bad_path() {
        let dir = crate::testing::temp_dir("store");
        let path = dir.join("flow.db");
        Store::open(&path).unwrap().set("a", "b").unwrap();
        assert_eq!(Store::open(&path).unwrap().get("a").as_deref(), Some("b"));
        assert!(Store::open(&dir.join("missing/flow.db")).is_err());
    }

    #[test]
    fn history_search_last_and_edits() {
        let store = Store::memory();
        let first = store.add_dictation(&dictation("Hello Asha", "Slack", 100)).unwrap();
        let mut second = dictation("Budget is 50% done", "Excel", 200);
        second.category = Category::Work;
        second.outcome = Outcome::Copied;
        let second_id = store.add_dictation(&second).unwrap();
        store
            .add_dictation(&Dictation {
                text: String::new(),
                outcome: Outcome::Cancelled,
                ..dictation("", "Slack", 300)
            })
            .unwrap();
        store
            .add_dictation(&Dictation {
                outcome: Outcome::Failed,
                ..dictation("x", "Mail", 250)
            })
            .unwrap();

        let all = store.history("", 10, 0).unwrap();
        assert_eq!(all.len(), 4);
        assert_eq!(all[0].outcome, Outcome::Failed);
        assert_eq!(all[2].category, Category::Work);
        assert_eq!(all[2].outcome, Outcome::Copied);
        assert_eq!(store.history("", 1, 1).unwrap()[0].outcome, Outcome::Cancelled);
        assert_eq!(store.history("asha", 10, 0).unwrap().len(), 1);
        assert_eq!(store.history("excel", 10, 0).unwrap().len(), 1);
        assert_eq!(store.history("50%", 10, 0).unwrap().len(), 1);
        assert_eq!(store.history("5_%", 10, 0).unwrap().len(), 0);
        assert_eq!(store.last().unwrap().text, "x");

        store.set_edited(first, "Hello Aisha").unwrap();
        assert_eq!(store.dictation(first).unwrap().edited.as_deref(), Some("Hello Aisha"));
        store.set_edited(first, "Hello Asha").unwrap();
        assert_eq!(store.dictation(first).unwrap().edited, None);
        store.set_flagged(first, true).unwrap();
        store.set_outcome(second_id, Outcome::Pasted).unwrap();
        assert!(store.dictation(first).unwrap().flagged);
        assert_eq!(store.dictation(second_id).unwrap().outcome, Outcome::Pasted);
        assert_eq!(store.dictation(999).unwrap_err(), "That dictation no longer exists");
        assert!(store.set_edited(999, "x").is_err());
        assert!(Store::memory().last().is_none());
    }

    #[test]
    fn deleting_and_retention_return_audio_files() {
        let store = Store::memory();
        let old = store
            .add_dictation(&Dictation {
                audio: "old.wav".into(),
                ..dictation("old", "A", NOW.now - 90_000)
            })
            .unwrap();
        store
            .add_dictation(&Dictation {
                audio: "new.wav".into(),
                ..dictation("new", "A", NOW.now - 60)
            })
            .unwrap();
        store.add_dictation(&dictation("no audio", "A", NOW.now - 30)).unwrap();
        assert!(store.prune(Retention::Keep, NOW).unwrap().is_empty());
        assert_eq!(store.prune(Retention::Day, NOW).unwrap(), vec!["old.wav"]);
        assert!(store.dictation(old).is_err());
        assert_eq!(store.delete_dictations(Some(&[old, 2])).unwrap(), vec!["new.wav"]);
        assert_eq!(store.all().unwrap().len(), 1);
        store
            .add_dictation(&Dictation {
                audio: "x.wav".into(),
                ..dictation("x", "A", NOW.now)
            })
            .unwrap();
        assert_eq!(store.prune(Retention::Never, NOW).unwrap(), vec!["x.wav"]);
        store
            .add_dictation(&Dictation {
                audio: "y.wav".into(),
                ..dictation("y", "A", NOW.now)
            })
            .unwrap();
        assert_eq!(store.delete_dictations(None).unwrap(), vec!["y.wav"]);
        assert!(store.all().unwrap().is_empty());
    }

    #[test]
    fn dictionary_words_corrections_and_forgetting() {
        let store = Store::memory();
        assert!(store.add_word("", "Kubernetes", "manual", NOW).unwrap());
        assert!(store.add_word("Sumit Bat", "Sumedh Bhat", "manual", NOW).unwrap());
        assert!(!store.add_word("sumit bat", "Sumedh Bhat", "manual", NOW).unwrap());
        assert!(store.add_word("cube control", "kubectl", "teacher", NOW).unwrap());
        assert_eq!(store.add_word("", " ", "manual", NOW).unwrap_err(), "Enter a word");
        assert_eq!(
            store.add_word("same", "same", "manual", NOW).unwrap_err(),
            "The correction is the same as the original"
        );
        assert!(store
            .add_word("", &"x".repeat(61), "manual", NOW)
            .unwrap_err()
            .contains("60 characters"));

        let words = store.dictionary().unwrap();
        assert_eq!(
            words.iter().map(|w| w.term.as_str()).collect::<Vec<_>>(),
            vec!["kubectl", "Kubernetes", "Sumedh Bhat"]
        );
        assert_eq!(store.vocabulary(), vec!["kubectl", "Kubernetes", "Sumedh Bhat"]);

        // A deleted learned word is never learned again, but can be added by hand.
        store.remove_word(words[0].id).unwrap();
        store.remove_word(9999).unwrap();
        assert!(store.is_forgotten("Cube Control", "kubectl"));
        assert!(!store.add_word("cube control", "kubectl", "teacher", NOW).unwrap());
        assert!(store.add_word("cube control", "kubectl", "manual", NOW).unwrap());
        assert!(!store.is_forgotten("cube control", "kubectl"));
    }

    #[test]
    fn snippets_validate_and_update() {
        let store = Store::memory();
        let id = store.save_snippet(None, " my address ", "221B Baker Street").unwrap();
        assert_eq!(
            store.save_snippet(None, "my address", "x").unwrap_err(),
            "There is already a snippet for \"my address\""
        );
        assert_eq!(
            store.save_snippet(None, "", "x").unwrap_err(),
            "A snippet needs both a phrase and its text"
        );
        assert!(store
            .save_snippet(None, &"p".repeat(61), "x")
            .unwrap_err()
            .contains("60"));
        assert!(store
            .save_snippet(None, "long", &"x".repeat(4001))
            .unwrap_err()
            .contains("4000"));
        assert_eq!(store.save_snippet(Some(id), "home address", "12 MG Road").unwrap(), id);
        let other = store
            .save_snippet(None, "calendar link", "https://cal.example/me")
            .unwrap();
        assert!(store.save_snippet(Some(other), "home address", "dup").is_err());
        assert_eq!(
            store.snippets().unwrap()[1],
            Snippet {
                id,
                trigger: "home address".into(),
                expansion: "12 MG Road".into(),
                hits: 0
            }
        );
        store.delete_snippet(id).unwrap();
        assert_eq!(store.snippets().unwrap().len(), 1);
    }

    #[test]
    fn rules_combine_dictionary_and_snippets_and_count_hits() {
        let store = Store::memory();
        store.add_word("", "GitHub", "manual", NOW).unwrap();
        store.add_word("sumit", "Sumedh", "manual", NOW).unwrap();
        store.save_snippet(None, "my email", "me@example.com").unwrap();
        let rules = store.replacement_rules();
        assert_eq!(
            rules.replacements,
            vec![
                Replacement::new("GitHub", "GitHub"),
                Replacement::new("sumit", "Sumedh"),
                Replacement::new("my email", "me@example.com")
            ]
        );
        store.record_hits(&rules, &[1, 2, 2, 7]).unwrap();
        assert_eq!(store.dictionary().unwrap()[1].hits, 1);
        assert_eq!(store.snippets().unwrap()[0].hits, 2);
    }

    #[test]
    fn notes_are_newest_first() {
        let store = Store::memory();
        let a = store.save_note(None, "A", "first", NOW).unwrap();
        let b = store
            .save_note(
                None,
                "B",
                "second",
                Clock {
                    now: NOW.now + 5,
                    ..NOW
                },
            )
            .unwrap();
        assert_eq!(
            store.notes().unwrap().iter().map(|n| n.id).collect::<Vec<_>>(),
            vec![b, a]
        );
        store
            .save_note(
                Some(a),
                "A2",
                "edited",
                Clock {
                    now: NOW.now + 9,
                    ..NOW
                },
            )
            .unwrap();
        assert_eq!(store.notes().unwrap()[0].title, "A2");
        store.delete_note(b).unwrap();
        assert_eq!(store.notes().unwrap().len(), 1);
    }

    #[test]
    fn suggestions_accumulate_then_resolve() {
        let store = Store::memory();
        assert_eq!(store.suggest("Sumit", "Sumedh", 1).unwrap(), 1);
        assert_eq!(store.suggest("sumit", "Sumedh", 2).unwrap(), 3);
        store.suggest("bat", "Bhat", 1).unwrap();
        let open = store.suggestions().unwrap();
        assert_eq!(
            open[0],
            Suggestion {
                id: open[0].id,
                heard: "sumit".into(),
                term: "Sumedh".into(),
                count: 3
            }
        );
        store.resolve_suggestion(open[0].id, true, "teacher", NOW).unwrap();
        store.resolve_suggestion(open[1].id, false, "teacher", NOW).unwrap();
        store.resolve_suggestion(999, true, "teacher", NOW).unwrap();
        assert!(store.suggestions().unwrap().is_empty());
        assert_eq!(store.dictionary().unwrap()[0].source, "teacher");
        // Known and rejected corrections stop counting.
        assert_eq!(store.suggest("sumit", "Sumedh", 1).unwrap(), 0);
        assert_eq!(store.suggest("bat", "Bhat", 1).unwrap(), 0);
    }

    #[test]
    fn reviews_and_examples() {
        let store = Store::memory();
        let with_audio = store
            .add_dictation(&Dictation {
                audio: "a.wav".into(),
                ..dictation("hello", "A", 1)
            })
            .unwrap();
        store.add_dictation(&dictation("no audio", "A", 2)).unwrap();
        store
            .add_dictation(&Dictation {
                audio: "c.wav".into(),
                outcome: Outcome::Cancelled,
                ..dictation("", "A", 3)
            })
            .unwrap();
        assert_eq!(
            store
                .pending_review(10)
                .unwrap()
                .iter()
                .map(|d| d.id)
                .collect::<Vec<_>>(),
            vec![with_audio]
        );
        store
            .save_review(with_audio, "hello there", "Hello there.", 0.5, NOW)
            .unwrap();
        let reviewed = store.dictation(with_audio).unwrap();
        assert_eq!(
            (
                reviewed.teacher_raw.as_deref(),
                reviewed.teacher_wer,
                reviewed.reviewed_at
            ),
            (Some("hello there"), Some(0.5), Some(NOW.now))
        );
        assert!(store.pending_review(10).unwrap().is_empty());

        store.add_example(Category::Email, "raw one", "Gold one.", NOW).unwrap();
        store.add_example(Category::Work, "raw two", "Gold two.", NOW).unwrap();
        store
            .add_example(Category::Email, "raw one", "Gold one, revised.", NOW)
            .unwrap();
        assert_eq!(
            store.examples(Category::Work, 1),
            vec![("raw two".into(), "Gold two.".into())]
        );
        assert_eq!(store.examples(Category::Email, 5)[0].1, "Gold one, revised.");
        for i in 0..205 {
            store
                .add_example(Category::Other, &format!("raw {i}"), "gold", NOW)
                .unwrap();
        }
        assert_eq!(store.examples(Category::Other, 500).len(), 200);
    }
}
