//! Recording history, retained audio and human corrections.
use super::{edits, now, Store};
use crate::config::owned;
use crate::config::{err, Result};
use rusqlite::params;
use serde_json::{json, Value};
use std::path::PathBuf;

impl Store {
    pub fn record(&self, audio: &[f32], raw: &str, pasted: &str, keep: bool) -> Result<i64> {
        let name = format!("{}-{}.wav", (now() * 1e6) as u64, std::process::id());
        if keep {
            crate::audio::save(&self.root.join("audio").join(&name), audio)?;
        }
        self.db
            .execute(
                "INSERT INTO samples(ts,audio,raw,pasted) VALUES(?,?,?,?)",
                params![now(), if keep { Some(name) } else { None }, raw, pasted],
            )
            .map_err(err)?;
        let id = self.db.last_insert_rowid();
        // One ten-minute clip at 16 kHz, at most 38.4 MB, never persisted until correction.
        *self.latest_audio.borrow_mut() = if !keep && audio.len() <= 16000 * 600 {
            Some((id, audio.to_vec()))
        } else {
            None
        };
        Ok(id)
    }
    pub fn history(&self, search: &str, filter: &str) -> Result<Vec<Value>> {
        self.history_rows(search, filter, 1000)
    }
    pub(super) fn history_rows(
        &self,
        search: &str,
        filter: &str,
        limit: i64,
    ) -> Result<Vec<Value>> {
        let mut statement = self
            .db
            .prepare(
                "SELECT id, ts, audio, raw, pasted, corrected, excluded,
                    teacher_text, teacher_confidence, teacher_note, teacher_model
             FROM samples
             WHERE coalesce(corrected, pasted) LIKE ?
               AND (? != 'edited' OR corrected IS NOT NULL)
               AND (? != 'unedited' OR corrected IS NULL)
               AND (? != 'excluded' OR excluded = 1)
             ORDER BY ts DESC LIMIT ?",
            )
            .map_err(err)?;
        let rows = statement
            .query_map(
                params![format!("%{search}%"), filter, filter, filter, limit],
                |row| {
                    Ok(json!({
                        "id": row.get::<_, i64>(0)?,
                        "ts": row.get::<_, f64>(1)?,
                        "audio": row.get::<_, Option<String>>(2)?,
                        "raw": row.get::<_, String>(3)?,
                        "pasted": row.get::<_, String>(4)?,
                        "corrected": row.get::<_, Option<String>>(5)?,
                        "excluded": row.get::<_, i32>(6)?,
                        "teacher_text": row.get::<_, Option<String>>(7)?,
                        "teacher_confidence": row.get::<_, Option<f64>>(8)?,
                        "teacher_note": row.get::<_, Option<String>>(9)?,
                        "teacher_model": row.get::<_, Option<String>>(10)?,
                    }))
                },
            )
            .map_err(err)?;
        rows.collect::<std::result::Result<_, _>>().map_err(err)
    }
    pub fn edit(&mut self, id: i64, text: &str) -> Result<()> {
        if text.trim().is_empty() {
            return Err("Correction cannot be empty".into());
        }
        let (before, pasted): (String, String) = self
            .db
            .query_row(
                "SELECT coalesce(corrected,pasted),pasted FROM samples WHERE id=?",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(err)?;
        let retained = self
            .latest_audio
            .borrow()
            .as_ref()
            .filter(|(sample, _)| *sample == id)
            .map(|(_, audio)| audio.clone());
        if let Some(audio) = retained {
            let name = format!("corrected-{id}.wav");
            let path = self.root.join("audio").join(&name);
            crate::audio::save(&path, &audio)?;
            if let Err(e) = self
                .db
                .execute("UPDATE samples SET audio=? WHERE id=?", params![name, id])
            {
                let _ = std::fs::remove_file(path);
                return Err(err(e));
            }
            self.latest_audio.borrow_mut().take();
        }
        if before == text {
            return Ok(());
        }
        let tx = self.db.transaction().map_err(err)?;
        tx.execute(
            "UPDATE samples SET corrected=? WHERE id=?",
            params![if pasted == text { None } else { Some(text) }, id],
        )
        .map_err(err)?;
        for (a, b) in edits(&before, text) {
            tx.execute("INSERT INTO rules VALUES(?,?,1) ON CONFLICT(wrong,right) DO UPDATE SET count=count+1",params![a,b]).map_err(err)?;
            tx.execute(
                "INSERT INTO vocab VALUES(?,1) ON CONFLICT(term) DO UPDATE SET count=count+1",
                [b],
            )
            .map_err(err)?;
        }
        tx.commit().map_err(err)
    }
    pub fn revert(&self, id: i64) -> Result<()> {
        self.db
            .execute("UPDATE samples SET corrected=NULL WHERE id=?", [id])
            .map_err(err)?;
        Ok(())
    }
    pub fn exclude(&self, id: i64, value: bool) -> Result<()> {
        self.db
            .execute(
                "UPDATE samples SET excluded=? WHERE id=?",
                params![value, id],
            )
            .map_err(err)?;
        Ok(())
    }
    pub fn audio_path(&self, id: i64) -> Result<PathBuf> {
        let name: String = self
            .db
            .query_row("SELECT audio FROM samples WHERE id=?", [id], |r| r.get(0))
            .map_err(err)?;
        owned(&self.root.join("audio"), &name)
    }
    pub fn delete(&self, id: i64) -> Result<()> {
        if self
            .latest_audio
            .borrow()
            .as_ref()
            .is_some_and(|(sample, _)| *sample == id)
        {
            self.latest_audio.borrow_mut().take();
        }
        if let Ok(p) = self.audio_path(id) {
            if p.exists() {
                std::fs::remove_file(p).map_err(err)?;
            }
        }
        self.db
            .execute("DELETE FROM samples WHERE id=?", [id])
            .map_err(err)?;
        Ok(())
    }
}
