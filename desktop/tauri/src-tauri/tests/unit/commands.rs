use super::commands::dispatch;
use crate::{keys::Action, service::Service};
use serde_json::json;
use std::{
    sync::{atomic::Ordering, mpsc, Arc},
    time::Instant,
};
fn service() -> Arc<Service> {
    let root = std::env::temp_dir().join(format!(
        "speechify-dispatch-{}-{}",
        std::process::id(),
        (crate::store::now() * 1e9) as u64
    ));
    for sub in ["audio", "models", "logs", "exports"] {
        std::fs::create_dir_all(root.join(sub)).unwrap();
    }
    Service::new(root, std::path::PathBuf::from("unused-fixture-native")).unwrap()
}
#[test]
fn refused_replacement_still_saves_correction_and_latest_audio() {
    let s = service();
    let sample = s
        .store
        .lock()
        .unwrap()
        .record(&[0.1; 160], "old", "old", false)
        .unwrap();
    let error = dispatch(
        &s,
        "correct",
        &json!({"id":sample,"text":"human correction","replace":true}),
    )
    .unwrap_err();
    assert!(error.contains("Correction saved"));
    let rows = dispatch(&s, "history", &json!({})).unwrap();
    assert_eq!(rows[0]["corrected"], "human correction");
    assert!(s
        .store
        .lock()
        .unwrap()
        .audio_path(sample)
        .unwrap()
        .is_file());
    let root = s.root.clone();
    drop(s);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn older_correction_does_not_consume_latest_replacement_receipt() {
    let s = service();
    let older = s
        .store
        .lock()
        .unwrap()
        .record(&[], "older", "older", false)
        .unwrap();
    let latest = s
        .store
        .lock()
        .unwrap()
        .record(&[], "latest", "latest", false)
        .unwrap();
    *s.replacement.lock().unwrap() = Some((
        latest,
        crate::replacement::Receipt {
            target: 0,
            expected: vec![],
            start: 0,
            end: 0,
            process: 0,
            created: Instant::now(),
        },
    ));
    assert!(dispatch(
        &s,
        "correct",
        &json!({"id":older,"text":"older corrected","replace":true})
    )
    .is_err());
    assert_eq!(s.replacement.lock().unwrap().as_ref().unwrap().0, latest);
    let root = s.root.clone();
    drop(s);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn invalid_install_does_not_latch_download_and_manual_tap_cancels() {
    let s = service();
    assert!(dispatch(&s, "install", &json!({"id":"not-a-model"})).is_err());
    assert!(!s.downloading.load(Ordering::SeqCst));
    assert!(dispatch(&s, "manual_start", &json!({})).is_err());
    assert!(!s.recording.load(Ordering::SeqCst));
    let (tx, rx) = mpsc::sync_channel(4);
    *s.capture_tx.lock().unwrap() = Some(tx);
    dispatch(&s, "manual_start", &json!({})).unwrap();
    assert_eq!(rx.recv().unwrap().action, Action::Start);
    dispatch(&s, "manual_stop", &json!({})).unwrap();
    assert_eq!(rx.recv().unwrap().action, Action::Cancel);
    assert!(!s.recording.load(Ordering::SeqCst));
    dispatch(&s, "manual_start", &json!({})).unwrap();
    rx.recv().unwrap();
    dispatch(&s, "cancel", &json!({})).unwrap();
    assert_eq!(rx.recv().unwrap().action, Action::Cancel);
    assert!(s.manual_started.lock().unwrap().is_none());
    let root = s.root.clone();
    drop(s);
    std::fs::remove_dir_all(root).unwrap();
}
