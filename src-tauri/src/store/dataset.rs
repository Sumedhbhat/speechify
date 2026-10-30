//! Export approved labels and import a consistent backup of an existing data directory.
use super::{label, now, schema::migrate, Store};
use crate::config::{err, owned, Result};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

impl Store {
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
            return Err("Import requires empty history".into());
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
            // Migrate the staged snapshot before touching current history.
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
