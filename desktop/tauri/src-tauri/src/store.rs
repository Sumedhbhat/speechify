use crate::config::{owned, Result};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
pub struct Store {
    pub db: Connection,
    pub root: PathBuf,
    latest_audio: std::cell::RefCell<Option<(i64, Vec<f32>)>>,
}
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
pub fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
fn migrate(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS samples(id INTEGER PRIMARY KEY,ts REAL NOT NULL,audio TEXT,raw TEXT NOT NULL,pasted TEXT NOT NULL,corrected TEXT,excluded INTEGER NOT NULL DEFAULT 0);CREATE TABLE IF NOT EXISTS rules(wrong TEXT NOT NULL,right TEXT NOT NULL,count INTEGER NOT NULL,PRIMARY KEY(wrong,right));CREATE TABLE IF NOT EXISTS vocab(term TEXT PRIMARY KEY,count INTEGER NOT NULL);CREATE TABLE IF NOT EXISTS learn_state(key TEXT PRIMARY KEY,value TEXT NOT NULL);").map_err(err)?;
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
    fn history_rows(&self, search: &str, filter: &str, limit: i64) -> Result<Vec<Value>> {
        let mut st=self.db.prepare("SELECT id,ts,audio,raw,pasted,corrected,excluded,teacher_text,teacher_confidence,teacher_note,teacher_model FROM samples WHERE coalesce(corrected,pasted) LIKE ? AND (? != 'edited' OR corrected IS NOT NULL) AND (? != 'unedited' OR corrected IS NULL) AND (? != 'excluded' OR excluded=1) ORDER BY ts DESC LIMIT ?").map_err(err)?;
        let rows=st.query_map(params![format!("%{search}%"),filter,filter,filter,limit],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"ts":r.get::<_,f64>(1)?,"audio":r.get::<_,Option<String>>(2)?,"raw":r.get::<_,String>(3)?,"pasted":r.get::<_,String>(4)?,"corrected":r.get::<_,Option<String>>(5)?,"excluded":r.get::<_,i32>(6)?,"teacher_text":r.get::<_,Option<String>>(7)?,"teacher_confidence":r.get::<_,Option<f64>>(8)?,"teacher_note":r.get::<_,Option<String>>(9)?,"teacher_model":r.get::<_,Option<String>>(10)?}))).map_err(err)?;
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
    pub fn learning(&self) -> Result<Value> {
        let rules=self.db.prepare("SELECT wrong,right,count FROM rules ORDER BY count DESC").map_err(err)?.query_map([],|r|Ok(json!({"wrong":r.get::<_,String>(0)?,"right":r.get::<_,String>(1)?,"count":r.get::<_,i64>(2)?}))).map_err(err)?.collect::<std::result::Result<Vec<_>,_>>().map_err(err)?;
        let vocab = self
            .db
            .prepare("SELECT term,count FROM vocab ORDER BY count DESC")
            .map_err(err)?
            .query_map([], |r| {
                Ok(json!({"term":r.get::<_,String>(0)?,"count":r.get::<_,i64>(1)?}))
            })
            .map_err(err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(err)?;
        Ok(
            json!({"rules":rules,"vocab":vocab,"versions":self.get_state("versions").unwrap_or(json!([])),"teacher":self.get_state("teacher_status").unwrap_or(json!("Idle")),"training":self.get_state("training_status").unwrap_or(json!("Not started"))}),
        )
    }
    pub fn forget(&self, kind: &str, key: &str) -> Result<()> {
        match kind {
            "rule" => {
                self.db
                    .execute("DELETE FROM rules WHERE wrong=?", [key])
                    .map_err(err)?;
            }
            "vocab" => {
                self.db
                    .execute("DELETE FROM vocab WHERE term=?", [key])
                    .map_err(err)?;
            }
            _ => return Err("Unknown learning item".into()),
        }
        Ok(())
    }
    pub fn apply(&self, text: &str, threshold: u64) -> Result<String> {
        let mut rules=self.db.prepare("SELECT wrong,right FROM rules WHERE count>=? ORDER BY length(wrong) DESC,count DESC").map_err(err)?;
        let rows = rules
            .query_map([threshold as i64], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(err)?;
        if rows.is_empty() {
            return Ok(text.into());
        }
        let mut mapping = std::collections::HashMap::new();
        let mut patterns = Vec::new();
        for (a, b) in rows {
            if mapping.contains_key(&a) {
                continue;
            }
            patterns.push(regex::escape(&a).replace(' ', r"\s+"));
            mapping.insert(a, b);
        }
        let re = regex::Regex::new(&format!(r"(?i)\b(?:{})\b", patterns.join("|"))).map_err(err)?;
        Ok(re
            .replace_all(text, |cap: &regex::Captures| {
                let key = cap[0]
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .to_lowercase();
                mapping.get(&key).cloned().unwrap_or_else(|| cap[0].into())
            })
            .into_owned())
    }
    pub fn prompt(&self) -> Result<String> {
        let terms = self
            .db
            .prepare("SELECT term FROM vocab ORDER BY count DESC,term")
            .map_err(err)?
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(err)?;
        let mut p = String::new();
        for t in terms {
            if p.len() + t.len() + if p.is_empty() { 0 } else { 2 } > 450 {
                break;
            }
            if !p.is_empty() {
                p.push_str(", ");
            }
            p.push_str(&t);
        }
        Ok(p)
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
        self.db.execute("INSERT INTO learn_state VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![k,v.to_string()]).map_err(err)?;
        Ok(())
    }
    pub fn export(&self, threshold: f64) -> Result<PathBuf> {
        let out = self
            .root
            .join("exports")
            .join(format!("dataset-{}", (now() * 1e6) as u64));
        std::fs::create_dir_all(&out).map_err(err)?;
        let mut csv = "file_name,transcription,source\n".to_string();
        for r in self.history_rows("", "", -1)? {
            if let Some((text, source)) = label(&r, threshold) {
                let src = self.audio_path(r["id"].as_i64().unwrap())?;
                if src.exists() {
                    let name = src.file_name().unwrap().to_string_lossy();
                    std::fs::copy(&src, out.join(name.as_ref())).map_err(err)?;
                    csv.push_str(&format!(
                        "\"{}\",\"{}\",{}\n",
                        name,
                        text.replace('"', "\"\""),
                        source
                    ));
                }
            }
        }
        std::fs::write(out.join("metadata.csv"), csv).map_err(err)?;
        Ok(out)
    }
    pub fn import(&mut self, source: &Path) -> Result<usize> {
        if self
            .db
            .query_row("SELECT count(*) FROM samples", [], |r| r.get::<_, i64>(0))
            .map_err(err)?
            > 0
        {
            return Err("Import requires empty candidate history".into());
        }
        let stage = self.root.join("import.db");
        let mut copied = Vec::new();
        let result = (|| {
            let src = Connection::open_with_flags(
                source.join("learning.db"),
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .map_err(err)?;
            let mut dest = Connection::open(&stage).map_err(err)?;
            {
                let backup = rusqlite::backup::Backup::new(&src, &mut dest).map_err(err)?;
                backup
                    .run_to_completion(50, std::time::Duration::from_millis(10), None)
                    .map_err(err)?;
            }
            // Migrate the staged snapshot before touching candidate history.
            migrate(&dest)?;
            let names = dest
                .prepare("SELECT DISTINCT audio FROM samples WHERE audio IS NOT NULL")
                .map_err(err)?
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(err)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(err)?;
            for name in &names {
                let p = owned(&source.join("audio"), name)?;
                let q = owned(&self.root.join("audio"), name)?;
                let mut input = std::fs::File::open(p).map_err(err)?;
                let mut output = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&q)
                    .map_err(err)?;
                copied.push(q);
                std::io::copy(&mut input, &mut output).map_err(err)?;
                output.sync_all().map_err(err)?;
            }
            let backup = rusqlite::backup::Backup::new(&dest, &mut self.db).map_err(err)?;
            backup
                .run_to_completion(50, std::time::Duration::from_millis(10), None)
                .map_err(err)?;
            Ok(names.len())
        })();
        if result.is_err() {
            for path in copied {
                let _ = std::fs::remove_file(path);
            }
        } else {
            self.latest_audio.borrow_mut().take();
        }
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", stage.display()));
        }
        result
    }
}
pub fn label(row: &Value, min: f64) -> Option<(String, &'static str)> {
    if row["excluded"].as_i64().unwrap_or(0) != 0 || !row["audio"].is_string() {
        return None;
    }
    if let Some(t) = row["corrected"].as_str().filter(|t| !t.trim().is_empty()) {
        return Some((t.into(), "corrected"));
    }
    if row["teacher_note"].is_null() && row["teacher_confidence"].as_f64().unwrap_or(0.) >= min {
        if let Some(t) = row["teacher_text"]
            .as_str()
            .filter(|t| !t.trim().is_empty())
        {
            return Some((t.into(), "teacher"));
        }
    }
    None
}
pub fn edits(a: &str, b: &str) -> Vec<(String, String)> {
    // LCS anchors separate bounded replacements from rewrites and insertions.
    let aa: Vec<_> = a.split_whitespace().collect();
    let bb: Vec<_> = b.split_whitespace().collect();
    if aa.len() > 512 || bb.len() > 512 {
        return vec![];
    }
    let norm = |s: &str| {
        s.trim_matches(|c: char| !c.is_alphanumeric())
            .to_lowercase()
    };
    let mut dp = vec![vec![0; bb.len() + 1]; aa.len() + 1];
    for i in (0..aa.len()).rev() {
        for j in (0..bb.len()).rev() {
            dp[i][j] = if norm(aa[i]) == norm(bb[j]) {
                1 + dp[i + 1][j + 1]
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out = vec![];
    while i < aa.len() && j < bb.len() {
        if norm(aa[i]) == norm(bb[j]) {
            let x = aa[i].trim_matches(|c: char| !c.is_alphanumeric());
            let y = bb[j].trim_matches(|c: char| !c.is_alphanumeric());
            if x != y {
                out.push((norm(x), y.into()));
            }
            i += 1;
            j += 1;
        } else {
            let (si, sj) = (i, j);
            while i < aa.len() && j < bb.len() && norm(aa[i]) != norm(bb[j]) {
                if dp[i + 1][j] >= dp[i][j + 1] {
                    i += 1;
                } else {
                    j += 1;
                }
            }
            if i == aa.len() {
                j = bb.len();
            }
            if j == bb.len() {
                i = aa.len();
            }
            if i > si && j > sj && i - si <= 4 && j - sj <= 4 {
                out.push((
                    aa[si..i]
                        .iter()
                        .map(|s| norm(s))
                        .collect::<Vec<_>>()
                        .join(" "),
                    bb[sj..j]
                        .join(" ")
                        .trim_matches(|c: char| !c.is_alphanumeric())
                        .into(),
                ));
            }
        }
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_guesses() {
        assert!(label(&json!({"audio":"a.wav","pasted":"guess"}), 0.7).is_none());
        assert_eq!(label(&json!({"audio":"a.wav","corrected":"human","teacher_text":"teacher","teacher_confidence":0.9}),0.7).unwrap().0,"human");
    }
    #[test]
    fn bounded_edits() {
        assert_eq!(
            edits("open get hub now", "open GitHub now"),
            vec![("get hub".into(), "GitHub".into())]
        );
        assert!(edits("one two three four five six", "a b c d e f").is_empty());
    }
}

#[cfg(test)]
mod persistence_tests {
    use super::*;
    #[test]
    fn restart_correction_export_and_safe_delete() {
        let root = std::env::temp_dir().join(format!(
            "speechify-tauri-test-{}-{}",
            std::process::id(),
            (now() * 1e9) as u64
        ));
        std::fs::create_dir_all(root.join("audio")).unwrap();
        std::fs::create_dir_all(root.join("exports")).unwrap();
        let mut s = Store::open(&root).unwrap();
        let id = s
            .record(&vec![0.2; 1600], "open get hub", "open get hub", true)
            .unwrap();
        s.edit(id, "open GitHub").unwrap();
        let id2 = s
            .record(&vec![0.2; 1600], "open get hub", "open get hub", true)
            .unwrap();
        s.edit(id2, "open GitHub").unwrap();
        assert_eq!(s.apply("open get hub", 2).unwrap(), "open GitHub");
        drop(s);
        let s = Store::open(&root).unwrap();
        assert_eq!(s.history("GitHub", "edited").unwrap().len(), 2);
        let export = s.export(0.7).unwrap();
        assert!(std::fs::read_to_string(export.join("metadata.csv"))
            .unwrap()
            .contains("corrected"));
        s.revert(id).unwrap();
        assert_eq!(s.history("", "unedited").unwrap().len(), 1);
        let audio = s.audio_path(id).unwrap();
        s.delete(id).unwrap();
        assert!(!audio.exists());
        s.db.execute(
            "UPDATE samples SET audio='../outside.wav' WHERE id=?",
            [id2],
        )
        .unwrap();
        assert!(s.audio_path(id2).is_err());
        drop(s);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn retention_off_keeps_only_latest_until_correction_or_restart() {
        let root =
            std::env::temp_dir().join(format!("speechify-retained-audio-{}", std::process::id()));
        std::fs::create_dir_all(root.join("audio")).unwrap();
        let mut s = Store::open(&root).unwrap();
        let old = s.record(&[0.1; 160], "old", "old", false).unwrap();
        let latest = s.record(&[0.2; 320], "latest", "latest", false).unwrap();
        assert_eq!(std::fs::read_dir(root.join("audio")).unwrap().count(), 0);
        s.edit(old, "old corrected").unwrap();
        assert!(s.audio_path(old).is_err());
        s.edit(latest, "latest corrected").unwrap();
        let path = s.audio_path(latest).unwrap();
        assert_eq!(crate::audio::load(&path).unwrap().len(), 320);
        assert!(s.latest_audio.borrow().is_none());
        let lost = s.record(&[0.3; 160], "lost", "lost", false).unwrap();
        drop(s);
        let mut s = Store::open(&root).unwrap();
        s.edit(lost, "lost corrected").unwrap();
        assert!(s.audio_path(lost).is_err());
        let deleted = s.record(&[0.1; 160], "delete", "delete", false).unwrap();
        s.delete(deleted).unwrap();
        assert!(s.latest_audio.borrow().is_none());
        drop(s);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn export_includes_labels_older_than_history_limit() {
        let root =
            std::env::temp_dir().join(format!("speechify-export-limit-{}", std::process::id()));
        std::fs::create_dir_all(root.join("audio")).unwrap();
        let mut s = Store::open(&root).unwrap();
        let id = s.record(&[0.1; 16], "old", "old", true).unwrap();
        s.edit(id, "corrected oldest").unwrap();
        s.db.execute("UPDATE samples SET ts=0 WHERE id=?", [id])
            .unwrap();
        for _ in 0..1001 {
            s.record(&[], "new", "new", false).unwrap();
        }
        assert_eq!(s.history("", "").unwrap().len(), 1000);
        let out = s.export(0.7).unwrap();
        assert!(std::fs::read_to_string(out.join("metadata.csv"))
            .unwrap()
            .contains("corrected oldest"));
        drop(s);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn legacy_import_migrates_and_failed_audio_copy_is_retryable() {
        let base =
            std::env::temp_dir().join(format!("speechify-legacy-import-{}", std::process::id()));
        let src = base.join("original");
        let dst = base.join("candidate");
        for p in [&src, &dst] {
            std::fs::create_dir_all(p.join("audio")).unwrap();
        }
        let legacy = Connection::open(src.join("learning.db")).unwrap();
        legacy.execute_batch("CREATE TABLE samples(id INTEGER PRIMARY KEY,ts REAL NOT NULL,audio TEXT,raw TEXT NOT NULL,pasted TEXT NOT NULL,corrected TEXT); INSERT INTO samples VALUES(1,0,'a.wav','old','old','human'); INSERT INTO samples VALUES(2,1,'missing.wav','missing','missing',NULL);").unwrap();
        crate::audio::save(&src.join("audio/a.wav"), &[0.1; 16]).unwrap();
        let mut dest = Store::open(&dst).unwrap();
        assert!(dest.import(&src).is_err());
        assert!(!dst.join("import.db").exists());
        assert_eq!(std::fs::read_dir(dst.join("audio")).unwrap().count(), 0);
        assert!(dest.history("", "").unwrap().is_empty());
        legacy
            .execute("DELETE FROM samples WHERE id=2", [])
            .unwrap();
        assert_eq!(dest.import(&src).unwrap(), 1);
        let row = &dest.history("", "").unwrap()[0];
        assert_eq!(row["excluded"], 0);
        assert_eq!(row["corrected"], "human");
        assert!(row["teacher_text"].is_null());
        let original_has_excluded: i64 = legacy
            .query_row(
                "SELECT count(*) FROM pragma_table_info('samples') WHERE name='excluded'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(original_has_excluded, 0);
        drop(dest);
        drop(legacy);
        std::fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn consistent_import_copies_audio() {
        let base = std::env::temp_dir().join(format!(
            "speechify-tauri-import-{}-{}",
            std::process::id(),
            (now() * 1e9) as u64
        ));
        let src = base.join("original");
        let dst = base.join("candidate");
        for p in [&src, &dst] {
            std::fs::create_dir_all(p.join("audio")).unwrap();
        }
        let source = Store::open(&src).unwrap();
        source
            .record(&[0.1; 1600], "original", "original", true)
            .unwrap();
        let mut dest = Store::open(&dst).unwrap();
        assert_eq!(dest.import(&src).unwrap(), 1);
        assert_eq!(dest.history("", "").unwrap().len(), 1);
        dest.delete(1).unwrap();
        assert!(source.audio_path(1).unwrap().exists());
        assert_eq!(source.history("", "").unwrap().len(), 1);
        drop(source);
        drop(dest);
        std::fs::remove_dir_all(base).unwrap();
    }
}
