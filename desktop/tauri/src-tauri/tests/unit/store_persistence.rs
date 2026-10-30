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
    let root = std::env::temp_dir().join(format!(
        "speechify-retained-audio-{}-{}",
        std::process::id(),
        (now() * 1e9) as u64
    ));
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
    let root = std::env::temp_dir().join(format!(
        "speechify-export-limit-{}-{}",
        std::process::id(),
        (now() * 1e9) as u64
    ));
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
    let base = std::env::temp_dir().join(format!(
        "speechify-legacy-import-{}-{}",
        std::process::id(),
        (now() * 1e9) as u64
    ));
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
