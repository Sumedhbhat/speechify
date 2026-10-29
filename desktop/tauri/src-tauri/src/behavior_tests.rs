//! Contracts exercised through the same APIs as the desktop command service.
use crate::{
    audio, config,
    keys::{Action, Hold},
    models,
    service::Service,
    store::{self, Store},
};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

struct Fixtures {
    _home: Home,
    native: PathBuf,
    worker: PathBuf,
}
fn fixtures() -> &'static Fixtures {
    static FIXTURES: std::sync::OnceLock<Fixtures> = std::sync::OnceLock::new();
    FIXTURES.get_or_init(|| {
        let home = Home::new();
        let native = home.0.join(if cfg!(windows) {
            "transcribe.dll"
        } else if cfg!(target_os = "macos") {
            "libtranscribe.dylib"
        } else {
            "libtranscribe.so"
        });
        let worker = home.0.join(if cfg!(windows) {
            "worker.exe"
        } else {
            "worker"
        });
        let declarations = include_str!("ffi.rs")
            .split("pub fn verify")
            .next()
            .unwrap();
        std::fs::write(
            home.0.join("native.rs"),
            format!(
                "{declarations}\n{}",
                include_str!("../../tests/fixtures/native.rs")
            ),
        )
        .unwrap();
        for (source, output, kind) in [
            (home.0.join("native.rs"), &native, "cdylib"),
            (
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/worker.rs"),
                &worker,
                "bin",
            ),
        ] {
            let mut command = std::process::Command::new("rustc");
            if let Ok(linker) = std::env::var("CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER") {
                command.arg("-C").arg(format!("linker={linker}"));
            }
            let status = command
                .arg("--edition=2021")
                .args(["--crate-type", kind])
                .arg(source)
                .arg("-o")
                .arg(output)
                .status()
                .unwrap();
            assert!(status.success(), "fixture compiler failed");
        }
        Fixtures {
            _home: home,
            native,
            worker,
        }
    })
}

fn fake_service(home: &Home, scenario: &str) -> std::sync::Arc<Service> {
    let model = home.0.join("models/whisper-ft-test");
    std::fs::create_dir_all(&model).unwrap();
    std::fs::write(model.join("model.bin"), "fixture model").unwrap();
    std::fs::write(model.join("scenario"), scenario).unwrap();
    let python = home.0.join(if cfg!(windows) {
        "environments/whisper/Scripts/python.exe"
    } else {
        "environments/whisper/bin/python"
    });
    std::fs::create_dir_all(python.parent().unwrap()).unwrap();
    std::fs::copy(&fixtures().worker, &python).unwrap();
    let mut cfg = config::defaults();
    cfg["model"] = json!("ft:whisper-ft-test");
    cfg["device"] = json!("cpu");
    config::save(&home.0, &cfg).unwrap();
    home.service()
}

static NEXT: AtomicUsize = AtomicUsize::new(0);
pub(crate) struct Home(pub PathBuf);
impl Home {
    pub(crate) fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "speechify-behavior-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        for sub in ["audio", "models", "logs", "exports", "resources/native"] {
            std::fs::create_dir_all(p.join(sub)).unwrap();
        }
        Self(p)
    }
    fn store(&self) -> Store {
        Store::open(&self.0).unwrap()
    }
    pub(crate) fn service(&self) -> std::sync::Arc<Service> {
        Service::new(self.0.clone(), self.0.join("resources/native")).unwrap()
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn settings_survive_restart_and_partial_import_keeps_defaults() {
    let home = Home::new();
    assert_eq!(config::load(&home.0).unwrap(), config::defaults());
    std::fs::write(
        home.0.join("config.toml"),
        "language = 'fr'\nunknown = 'preserved'\n",
    )
    .unwrap();
    let mut cfg = config::load(&home.0).unwrap();
    assert_eq!(cfg["language"], "fr");
    assert_eq!(cfg["beam_size"], 5);
    cfg["keep_mic_open"] = json!(true);
    config::save(&home.0, &cfg).unwrap();
    assert_eq!(config::load(&home.0).unwrap(), cfg);
    assert!(!home.0.join("config.tmp").exists());
    assert!(config::save(&home.0.join("missing"), &cfg).is_err());
    std::fs::write(home.0.join("config.toml"), "language = [").unwrap();
    assert!(config::load(&home.0).is_err());
}

#[test]
fn invalid_settings_never_replace_the_saved_configuration() {
    let home = Home::new();
    let cfg = config::defaults();
    config::save(&home.0, &cfg).unwrap();
    for (key, value) in [
        ("beam_size", json!(0)),
        ("beam_size", json!(33)),
        ("beam_size", json!(1.5)),
        ("min_hold_ms", json!(-1)),
        ("keep_mic_open", json!("false")),
        ("device", json!("tpu")),
        ("theme", json!("sepia")),
        ("compute_type", json!("int4")),
        ("teacher_min_confidence", json!(1.01)),
        ("idle_minutes", json!(0)),
        ("ptt_key", json!("F25")),
        ("correct_hotkey", json!("<alt>+<ctrl>+h")),
        ("llm_url", json!("file:///private/transcripts")),
        ("llm_url", json!("invalid url")),
    ] {
        let mut invalid = cfg.clone();
        invalid[key] = value.clone();
        assert!(
            config::save(&home.0, &invalid).is_err(),
            "accepted {key}={value}"
        );
        assert_eq!(config::load(&home.0).unwrap(), cfg);
    }
    let mut missing = cfg.clone();
    missing.as_object_mut().unwrap().remove("language");
    assert!(config::validate(&missing).unwrap_err().contains("language"));
    for url in [
        "http://localhost:11434/v1/chat/completions",
        "https://example.org/cleanup",
    ] {
        let mut valid = cfg.clone();
        valid["llm_url"] = json!(url);
        config::validate(&valid).unwrap();
    }
}

#[test]
fn candidate_files_and_lock_cannot_escape_or_share_another_data_root() {
    let home = Home::new();
    for name in [
        "",
        ".",
        "..",
        "../private",
        "audio/file.wav",
        "/private/file",
    ] {
        assert!(config::owned(&home.0, name).is_err(), "accepted {name}");
    }
    assert_eq!(
        config::owned(&home.0, "recording.wav").unwrap(),
        home.0.join("recording.wav")
    );
    #[cfg(unix)]
    {
        let outside = Home::new();
        std::fs::write(outside.0.join("private"), "private").unwrap();
        std::os::unix::fs::symlink(outside.0.join("private"), home.0.join("escape")).unwrap();
        assert!(config::owned(&home.0, "escape").is_err());
        let lock = config::lock(&home.0).unwrap();
        assert!(config::lock(&home.0).is_err());
        drop(lock);
        config::lock(&home.0).unwrap();
    }
    assert!(config::lock(&home.0.join("absent")).is_err());
}

#[test]
fn key_aliases_and_invalid_shortcuts_have_a_stable_contract() {
    for (input, normalized) in [
        (" <ALT> + <ctrl> + F ", "alt+ctrl+f"),
        ("ctrl+ctrl+f", "ctrl+f"),
        ("f24", "f24"),
        ("7", "7"),
    ] {
        assert_eq!(crate::keys::parse(input).unwrap(), normalized);
    }
    for bad in ["", "ctrl+", "hyper+f", "f0", "f25", "é", "ctrl+left+f"] {
        assert!(crate::keys::parse(bad).is_err(), "accepted {bad}");
    }
    let mut hold = Hold::new("ctrl+alt+f", 250);
    assert_eq!(hold.event("f", false, 0), None);
    assert_eq!(hold.event("shift_l", true, 1), None);
    assert_eq!(hold.event("f", true, 2), None);
    hold.event("f", false, 3);
    hold.event("shift_l", false, 4);
    hold.event("ctrl_r", true, 10);
    hold.event("alt_r", true, 11);
    assert_eq!(hold.event("f", true, 12), Some(Action::Start));
    assert_eq!(hold.event("unrelated", false, 13), None);
    assert_eq!(hold.event("ctrl_r", false, 262), Some(Action::Submit));
    hold.event("f", false, 263);
    hold.event("alt_r", false, 264);
    let mut hold = Hold::new("ctrl_r", 250);
    assert_eq!(hold.event("ctrl_r", true, 100), Some(Action::Start));
    assert_eq!(hold.event("ctrl_r", false, 350), Some(Action::Submit));
}

#[test]
fn wav_storage_clamps_samples_and_preserves_mono_audio() {
    let home = Home::new();
    let path = home.0.join("voice.wav");
    audio::save(&path, &[-2., -0.5, 0., 0.5, 2.]).unwrap();
    let clip = audio::load(&path).unwrap();
    assert_eq!(clip.len(), 5);
    for (actual, expected) in clip.iter().zip([-1., -0.5, 0., 0.5, 1.]) {
        assert!((actual - expected).abs() < 0.0001);
    }
    assert_eq!(
        audio::resample(&[1., -1., 0.4, 0.2, 0.9], 16000, 2),
        vec![0., 0.3]
    );
    assert!(audio::resample(&[1.], 0, 1).is_empty());
    assert!(audio::resample(&[1.], 16000, 0).is_empty());
    assert!(audio::resample(&[], 48000, 1).is_empty());
    assert!(audio::load(&home.0.join("missing.wav")).is_err());
    assert!(audio::save(&home.0.join("absent/voice.wav"), &[0.]).is_err());
}

#[test]
fn nonfinite_or_invalid_wav_files_are_rejected_before_inference() {
    let home = Home::new();
    let path = home.0.join("voice.wav");
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 8000,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut wav = hound::WavWriter::create(&path, spec).unwrap();
    for v in [0.3, 0.3, 0.6, 0.6] {
        wav.write_sample(v).unwrap();
    }
    wav.finalize().unwrap();
    let clip = audio::load(&path).unwrap();
    assert_eq!(clip.len(), 4);
    assert!(clip[0] > 0.2 && clip[3] > 0.4);
    let mut wav = hound::WavWriter::create(&path, spec).unwrap();
    wav.write_sample(f32::NAN).unwrap();
    wav.write_sample(0.).unwrap();
    wav.finalize().unwrap();
    assert!(audio::load(&path).unwrap_err().contains("Non-finite"));
    std::fs::write(&path, "not audio").unwrap();
    assert!(audio::load(&path).is_err());
}

#[test]
fn corrections_learn_repeated_rules_without_chaining_or_partial_word_matches() {
    let home = Home::new();
    let mut store = home.store();
    assert_eq!(store.apply("get hub", 2).unwrap(), "get hub");
    let a = store
        .record(&[0.1; 100], "get hub rocks", "get hub rocks", true)
        .unwrap();
    store.edit(a, "GitHub rocks").unwrap();
    assert_eq!(store.apply("get hub", 2).unwrap(), "get hub");
    assert_eq!(store.prompt().unwrap(), "GitHub");
    let b = store
        .record(&[0.1; 100], "get hub rocks", "get hub rocks", true)
        .unwrap();
    store.edit(b, "GitHub rocks").unwrap();
    assert_eq!(
        store.apply("GET   HUB and get hubbers", 2).unwrap(),
        "GitHub and get hubbers"
    );
    let c = store
        .record(&[], "GitHub rocks", "GitHub rocks", false)
        .unwrap();
    store.edit(c, "Codeberg rocks").unwrap();
    assert_eq!(
        store.apply("get hub and GitHub", 1).unwrap(),
        "GitHub and Codeberg"
    );
    let before = store.learning().unwrap();
    store.edit(b, "GitHub rocks").unwrap();
    assert_eq!(store.learning().unwrap(), before);
    store.forget("rule", "get hub").unwrap();
    assert_eq!(store.apply("get hub", 1).unwrap(), "get hub");
    store.forget("vocab", "GitHub").unwrap();
    assert_eq!(store.prompt().unwrap(), "Codeberg");
    assert!(store.forget("anything", "GitHub").is_err());
    assert!(store.edit(b, "  ").is_err());
    assert!(store.edit(99999, "not a sample").is_err());
}

#[test]
fn history_filtering_revert_and_deletion_survive_restart() {
    let home = Home::new();
    let mut store = home.store();
    let first = store
        .record(&[0.2; 160], "raw first", "First transcript", true)
        .unwrap();
    let second = store
        .record(&[0.2; 160], "raw second", "Second transcript", false)
        .unwrap();
    store.edit(first, "Corrected first").unwrap();
    store.exclude(second, true).unwrap();
    assert_eq!(
        store.history("Corrected", "edited").unwrap()[0]["id"],
        first
    );
    assert_eq!(store.history("", "unedited").unwrap()[0]["id"], second);
    assert_eq!(store.history("", "excluded").unwrap().len(), 1);
    assert!(store.history("missing", "").unwrap().is_empty());
    assert!(store.audio_path(second).is_err());
    store.state("teacher_status", &json!("Labelling")).unwrap();
    store.state("teacher_status", &json!("Done")).unwrap();
    drop(store);
    let store = home.store();
    assert_eq!(store.get_state("teacher_status").unwrap(), "Done");
    assert!(store.get_state("nonexistent").is_err());
    store.revert(first).unwrap();
    assert_eq!(store.history("", "unedited").unwrap().len(), 2);
    let path = store.audio_path(first).unwrap();
    store.delete(first).unwrap();
    assert!(!path.exists());
    store.delete(first).unwrap();
    store.delete(second).unwrap();
    assert!(store.history("", "").unwrap().is_empty());
}

#[test]
fn training_labels_never_use_uncorrected_dictation_guesses() {
    let base = json!({"audio":"voice.wav", "excluded":0, "corrected":null, "pasted":"student guess", "teacher_text":"teacher text", "teacher_confidence":0.8, "teacher_note":null});
    assert_eq!(
        store::label(&base, 0.7),
        Some(("teacher text".into(), "teacher"))
    );
    for (key, value) in [
        ("audio", Value::Null),
        ("excluded", json!(1)),
        ("teacher_confidence", json!(0.69)),
        ("teacher_text", json!("  ")),
        ("teacher_note", json!("repeated phrase")),
    ] {
        let mut row = base.clone();
        row[key] = value;
        assert!(store::label(&row, 0.7).is_none());
    }
    let mut row = base;
    row["corrected"] = json!("human correction");
    row["teacher_note"] = json!("rejected");
    assert_eq!(
        store::label(&row, 0.7),
        Some(("human correction".into(), "corrected"))
    );
    assert_eq!(
        store::edits("github is useful", "GitHub is useful"),
        vec![("github".into(), "GitHub".into())]
    );
    assert!(store::edits(&"word ".repeat(513), "new").is_empty());
    assert!(store::edits("hello", "hello there").is_empty());
    assert!(store::edits("hello there", "hello").is_empty());
}

#[test]
fn model_inventory_distinguishes_native_optional_and_trained_installations() {
    let home = Home::new();
    std::fs::write(models::path(&home.0, "moonshine-base").unwrap(), "weights").unwrap();
    std::fs::create_dir(home.0.join("models/kyutai-stt-1b")).unwrap();
    for name in ["whisper-ft-user", "incomplete", "whisper-ft-incomplete"] {
        std::fs::create_dir(home.0.join("models").join(name)).unwrap();
    }
    std::fs::write(home.0.join("models/whisper-ft-user/model.bin"), "weights").unwrap();
    std::fs::write(home.0.join("models/incomplete/model.bin"), "weights").unwrap();
    let list = models::list(&home.0);
    assert_eq!(list.len(), 15);
    assert_eq!(
        list.iter().find(|m| m["id"] == "moonshine-base").unwrap()["installed"],
        true
    );
    assert_eq!(
        list.iter().find(|m| m["id"] == "kyutai-stt-1b").unwrap()["installed"],
        true
    );
    assert_eq!(list.last().unwrap()["id"], "ft:whisper-ft-user");
    assert!(models::path(&home.0, "ft:../../other").is_err());
    for (id, family) in [
        ("ft:user", Some("whisper")),
        ("distil-whisper-large-v3", Some("whisper")),
        ("kyutai-stt-1b", Some("transformers")),
        ("moonshine-base", None),
    ] {
        assert_eq!(models::environment(id), family);
    }
    assert!(models::spec("unknown").is_err());
    assert!(models::verify(&home.0.join("absent"), "hash", &AtomicBool::new(false)).is_err());
    assert!(models::install(
        &home.0,
        "kyutai-stt-1b",
        std::sync::Arc::new(AtomicBool::new(false)),
        |_| {}
    )
    .is_err());
    assert_eq!(models::list(&home.0.join("absent")).len(), 14);
}

#[test]
fn startup_cleans_only_its_pending_wavs_and_rejects_a_second_service() {
    let home = Home::new();
    for name in ["pending-abandoned.wav", "keep.wav", "pending-not-audio.txt"] {
        std::fs::write(home.0.join(name), "data").unwrap();
    }
    std::fs::create_dir(home.0.join("pending-directory.wav")).unwrap();
    let service = home.service();
    assert!(!home.0.join("pending-abandoned.wav").exists());
    for name in ["keep.wav", "pending-not-audio.txt", "pending-directory.wav"] {
        assert!(home.0.join(name).exists());
    }
    #[cfg(unix)]
    assert!(Service::new(home.0.clone(), home.0.join("resources/native")).is_err());
    let snapshot = service.snapshot();
    assert_eq!(snapshot["state"]["stage"], "Idle");
    assert_eq!(snapshot["data_dir"], json!(home.0));
    service.error("No microphone");
    assert_eq!(service.snapshot()["state"]["error"], "No microphone");
    service.cancel_work();
    assert_eq!(service.snapshot()["state"]["stage"], "Cancelled");
    service.shutdown();
    assert!(service.preload().unwrap_err().contains("stopping"));
}

#[test]
fn settings_are_refused_during_recording_or_download_and_failed_model_selection_rolls_back() {
    let home = Home::new();
    let service = home.service();
    let initial = config::load(&home.0).unwrap();
    let mut changed = initial.clone();
    changed["language"] = json!("fr");
    service.recording.store(true, Ordering::SeqCst);
    assert!(service.update(changed.clone()).is_err());
    service.recording.store(false, Ordering::SeqCst);
    service.downloading.store(true, Ordering::SeqCst);
    assert!(service.update(changed.clone()).is_err());
    service.downloading.store(false, Ordering::SeqCst);
    assert!(service.update(changed).unwrap_err().contains("rolled back"));
    assert_eq!(config::load(&home.0).unwrap(), initial);
    assert_eq!(service.snapshot()["config"], initial);
    let mut changed = initial;
    changed["theme"] = json!("dark");
    service.update(changed.clone()).unwrap();
    assert_eq!(config::load(&home.0).unwrap(), changed);
}

#[test]
fn native_engine_honors_language_prompt_vocabulary_and_clip_limits() {
    let home = Home::new();
    std::fs::copy(
        &fixtures().native,
        home.0.join(fixtures().native.file_name().unwrap()),
    )
    .unwrap();
    for device in ["cpu", "vulkan", "metal", "cuda", "auto"] {
        let mut engine =
            crate::native::Native::open(&home.0, Path::new("whisper-vocab"), device, 2).unwrap();
        assert_eq!(engine.device, if device == "auto" { "cpu" } else { device });
        let result = engine
            .run(&vec![0.1; 16001], "en", "GitHub, kubectl")
            .unwrap();
        let chunks: Vec<_> = result.split(" 1|").collect();
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].starts_with("16000|en|Hello."));
        assert!(chunks[0].contains("Glossary: GitHub, kubectl."));
        assert!(chunks[0].ends_with("|GitHub,kubectl|pnc=2"));
        assert!(engine.run(&[], "en", "").unwrap().is_empty());
        assert!(engine.run(&[0.1], "en\0invalid", "").is_err());
        assert!(engine.run(&[0.1], "en", "bad\0prompt").is_err());
    }
    for (mode, expected) in [
        ("prompt", "1|fr|Bonjour|"),
        ("plain", "1|fr|||pnc=0"),
        ("no-languages", "1||||pnc=2"),
        ("empty-result", ""),
        ("unlimited", "1|fr|||pnc=2"),
        ("no-device", "1|fr|||pnc=2"),
        ("no-kind", "1|fr|||pnc=2"),
    ] {
        let mut engine = crate::native::Native::open(&home.0, Path::new(mode), "cpu", 1).unwrap();
        assert!(
            engine
                .run(&[0.1], "fr", "Bonjour")
                .unwrap()
                .starts_with(expected),
            "incorrect native contract for {mode}"
        );
    }
    for (mode, status) in [
        ("bad-load", 21),
        ("bad-session", 22),
        ("bad-caps", 24),
        ("bad-device", 25),
    ] {
        let error = crate::native::Native::open(&home.0, Path::new(mode), "cpu", 1)
            .err()
            .unwrap();
        assert!(error.contains(&status.to_string()));
    }
    let mut engine = crate::native::Native::open(&home.0, Path::new("bad-run"), "cpu", 1).unwrap();
    assert!(engine.run(&[0.1], "en", "").unwrap_err().contains("26"));
    assert!(
        crate::native::Native::open(&home.0.join("missing"), Path::new("model"), "cpu", 1).is_err()
    );
}

#[test]
fn incompatible_native_abi_is_rejected_before_loading_a_model() {
    let home = Home::new();
    let path = home.0.join(fixtures().native.file_name().unwrap());
    std::fs::copy(&fixtures().native, &path).unwrap();
    unsafe {
        let library = libloading::Library::new(&path).unwrap();
        let set = library
            .get::<unsafe extern "C" fn(i32)>(b"fixture_bad_abi\0")
            .unwrap();
        for id in 0..16 {
            set(id);
            assert!(
                crate::native::Native::open(&home.0, Path::new("model"), "cpu", 1)
                    .err()
                    .unwrap()
                    .contains("native ABI mismatch")
            );
        }
        set(-1);
        crate::native::Native::open(&home.0, Path::new("model"), "cpu", 1).unwrap();
    }
}

#[test]
fn dictation_flows_through_worker_rules_history_and_restart() {
    let home = Home::new();
    let service = fake_service(&home, "");
    service.preload().unwrap();
    assert_eq!(service.snapshot()["state"]["stage"], "Ready");
    let wav = home.0.join("input.wav");
    audio::save(&wav, &[0.1; 1600]).unwrap();
    let first = service.infer_path(&wav, true).unwrap();
    assert_eq!(first["text"], "get hub");
    let first_id = first["sample_id"].as_i64().unwrap();
    service
        .store
        .lock()
        .unwrap()
        .edit(first_id, "GitHub")
        .unwrap();
    let second = service.infer_path(&wav, true).unwrap();
    service
        .store
        .lock()
        .unwrap()
        .edit(second["sample_id"].as_i64().unwrap(), "GitHub")
        .unwrap();
    let third = service.infer_path(&wav, true).unwrap();
    assert_eq!(third["text"], "GitHub");
    let request: Value = serde_json::from_str(
        &std::fs::read_to_string(home.0.join("models/whisper-ft-test/request.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(request["language"], "en");
    assert_eq!(request["prompt"], "GitHub");
    assert_eq!(request["wav"], json!(wav));
    assert_eq!(service.snapshot()["state"]["recent"], "GitHub");
    let rows = service.store.lock().unwrap().history("", "").unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0]["raw"], "get hub");
    assert_eq!(rows[0]["pasted"], "GitHub");
    let logs = std::fs::read_to_string(home.0.join("logs/timings.jsonl")).unwrap();
    assert!(!logs.contains("get hub"));
    assert!(!logs.contains("GitHub"));
    drop(service);
    let store = home.store();
    assert_eq!(store.apply("get hub", 2).unwrap(), "GitHub");
}

#[test]
fn optional_cleanup_failure_preserves_valid_local_dictation() {
    let home = Home::new();
    let service = fake_service(&home, "");
    let mut cfg = service.config.lock().unwrap().clone();
    cfg["llm_url"] = json!("http://127.0.0.1:1/cleanup");
    cfg["llm_model"] = json!("cleanup");
    service.update(cfg).unwrap();
    let wav = home.0.join("input.wav");
    audio::save(&wav, &[0.1; 160]).unwrap();
    let result = service.infer_path(&wav, true).unwrap();
    assert_eq!(result["text"], "get hub");
    assert!(result["cleanup_warning"]
        .as_str()
        .unwrap()
        .contains("local transcription"));
    assert_eq!(
        service.store.lock().unwrap().history("", "").unwrap().len(),
        1
    );
}

#[test]
fn empty_recognition_is_not_saved_and_fixture_runs_do_not_add_history() {
    for scenario in ["", "empty"] {
        let home = Home::new();
        let service = fake_service(&home, scenario);
        let wav = home.0.join("input.wav");
        audio::save(&wav, &[0.1; 160]).unwrap();
        let result = service.infer_path(&wav, scenario == "empty").unwrap();
        assert!(result["sample_id"].is_null());
        assert!(service
            .store
            .lock()
            .unwrap()
            .history("", "")
            .unwrap()
            .is_empty());
    }
}

#[test]
fn worker_protocol_reports_bad_startup_errors_timeouts_and_releases_children() {
    for (scenario, expected) in [
        ("load-error", "model rejected"),
        ("not-ready", "false"),
        ("invalid", "Invalid worker response"),
        ("exit", "stopped responding"),
    ] {
        let home = Home::new();
        let service = fake_service(&home, scenario);
        assert!(
            service.preload().unwrap_err().contains(expected),
            "wrong failure for {scenario}"
        );
        assert_eq!(service.pid.load(Ordering::SeqCst), 0);
    }
    for scenario in ["error", "slow", "crash"] {
        let home = Home::new();
        let service = fake_service(&home, scenario);
        service.preload().unwrap();
        let wav = home.0.join("input.wav");
        audio::save(&wav, &[0.1; 160]).unwrap();
        let mut slot = service.worker.lock().unwrap();
        let worker = slot.as_mut().unwrap();
        assert!(worker.alive());
        let error = worker
            .run(&wav, "en", "", std::time::Duration::from_millis(50))
            .unwrap_err();
        assert!(error.contains(if scenario == "error" {
            "bad clip"
        } else {
            "stopped responding"
        }));
        slot.take();
        assert_eq!(service.pid.load(Ordering::SeqCst), 0);
    }
}
