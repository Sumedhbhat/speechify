//! Contracts exercised through the same APIs as the desktop command service.
use crate::{
    audio, config,
    keys::{Action, Hold},
    models,
    service::Service,
    store::{self, Store},
};
use serde_json::{json, Value};
#[path = "native.rs"]
mod native_behavior;
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
        let declarations = include_str!("../../src/ffi.rs")
            .split("pub fn verify")
            .next()
            .unwrap();
        std::fs::write(
            home.0.join("native.rs"),
            format!(
                "{declarations}\n{}",
                include_str!("../../../tests/fixtures/native.rs")
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

pub(crate) fn fake_service(home: &Home, scenario: &str) -> std::sync::Arc<Service> {
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

pub(crate) fn training_environment(home: &Home, report: &str) {
    let python = home.0.join(if cfg!(windows) {
        "environments/training/Scripts/python.exe"
    } else {
        "environments/training/bin/python"
    });
    std::fs::create_dir_all(python.parent().unwrap()).unwrap();
    std::fs::copy(&fixtures().worker, &python).unwrap();
    std::fs::write(
        home.0.join("environments/training/candidate-ready"),
        "cuda-cu126-v1",
    )
    .unwrap();
    std::fs::write(home.0.join("trainer-result.json"), report).unwrap();
}

#[track_caller]
pub(crate) fn wait_until(mut condition: impl FnMut() -> bool) {
    let start = std::time::Instant::now();
    while !condition() {
        assert!(
            start.elapsed() < std::time::Duration::from_secs(10),
            "behavior did not settle before deadline"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

static NEXT: AtomicUsize = AtomicUsize::new(0);
#[cfg(all(feature = "desktop", target_os = "linux"))]
pub(crate) static DESKTOP: std::sync::Mutex<()> = std::sync::Mutex::new(());
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
fn unreadable_settings_and_failed_atomic_replacement_preserve_the_previous_configuration() {
    let home = Home::new();
    let expected = config::defaults();
    config::save(&home.0, &expected).unwrap();
    std::fs::create_dir(home.0.join("config.tmp")).unwrap();
    assert!(config::save(&home.0, &expected).is_err());
    assert_eq!(config::load(&home.0).unwrap(), expected);
    std::fs::remove_dir(home.0.join("config.tmp")).unwrap();
    std::fs::remove_file(home.0.join("config.toml")).unwrap();
    std::fs::create_dir(home.0.join("config.toml")).unwrap();
    assert!(config::load(&home.0).is_err());
    assert!(config::save(&home.0, &expected).is_err());
    assert!(home.0.join("config.toml").is_dir());
}

#[test]
fn truncated_pcm_and_float_recordings_are_rejected_instead_of_transcribing_partial_audio() {
    let home = Home::new();
    for format in [hound::SampleFormat::Float, hound::SampleFormat::Int] {
        let path = home.0.join("truncated.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: if format == hound::SampleFormat::Float {
                32
            } else {
                16
            },
            sample_format: format,
        };
        let mut wav = hound::WavWriter::create(&path, spec).unwrap();
        if format == hound::SampleFormat::Float {
            wav.write_sample(0.5f32).unwrap();
        } else {
            wav.write_sample(500i16).unwrap();
        }
        wav.finalize().unwrap();
        let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_len(file.metadata().unwrap().len() - 1).unwrap();
        assert!(audio::load(&path).is_err());
    }
}

#[cfg(target_os = "linux")]
#[test]
fn full_storage_is_reported_for_both_empty_and_buffered_recordings() {
    let sink = Path::new("/dev/full");
    for audio in [Vec::new(), vec![0.25; 16000]] {
        assert!(audio::save(sink, &audio).is_err());
    }
}

#[test]
fn wav_duration_limits_use_wide_arithmetic_and_reject_recordings_over_ten_minutes() {
    let home = Home::new();
    let high_rate = home.0.join("high-rate.wav");
    let mut writer = hound::WavWriter::create(
        &high_rate,
        hound::WavSpec {
            channels: 1,
            sample_rate: 8_000_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for _ in 0..8000 {
        writer.write_sample(8192i16).unwrap();
    }
    writer.finalize().unwrap();
    let clip = audio::load(&high_rate).unwrap();
    assert_eq!(clip.len(), 16);
    assert!(clip.iter().all(|sample| (*sample - 0.25).abs() < 0.001));

    // A valid sparse PCM WAV represents 601 seconds of silence without
    // spending the test's time generating or transcribing ten minutes of audio.
    let path = home.0.join("over-limit.wav");
    let payload = 16000u32 * 601 * 2;
    let mut header = b"RIFF".to_vec();
    header.extend((payload + 36).to_le_bytes());
    header.extend(b"WAVEfmt ");
    header.extend(16u32.to_le_bytes());
    header.extend(1u16.to_le_bytes()); // PCM
    header.extend(1u16.to_le_bytes()); // mono
    header.extend(16000u32.to_le_bytes());
    header.extend(32000u32.to_le_bytes());
    header.extend(2u16.to_le_bytes());
    header.extend(16u16.to_le_bytes());
    header.extend(b"data");
    header.extend(payload.to_le_bytes());
    std::fs::write(&path, header).unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(u64::from(payload) + 44)
        .unwrap();
    assert!(audio::load(&path)
        .unwrap_err()
        .contains("at most 10 minutes"));
}

#[test]
fn deleting_the_only_unretained_recording_prevents_later_recovery_or_learning() {
    let home = Home::new();
    let mut store = home.store();
    let sample = store
        .record(&[0.1; 160], "raw", "unsaved voice", false)
        .unwrap();
    store.delete(sample).unwrap();
    assert!(store.history("", "").unwrap().is_empty());
    assert!(store.edit(sample, "should never be learned").is_err());
    assert!(!home
        .0
        .join(format!("audio/corrected-{sample}.wav"))
        .exists());
}

#[cfg(target_os = "linux")]
#[test]
fn helpers_are_refused_when_the_os_cannot_establish_parent_ownership() {
    use std::os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::process::CommandExt,
    };
    for (denied, expected) in [
        (libc::SYS_prctl, libc::EACCES),
        (libc::SYS_getppid, libc::ESRCH),
    ] {
        // The filter applies only in the forked child. A byte on this pipe
        // proves setup succeeded before the production ownership hook ran.
        let mut pipe = [-1; 2];
        assert_eq!(
            unsafe { libc::pipe2(pipe.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) },
            0
        );
        let reader = unsafe { OwnedFd::from_raw_fd(pipe[0]) };
        let writer = unsafe { OwnedFd::from_raw_fd(pipe[1]) };
        let fd = writer.as_raw_fd();
        let mut command = std::process::Command::new("/usr/bin/true");
        unsafe {
            command.pre_exec(move || {
                let filters = [
                    libc::sock_filter {
                        code: (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16,
                        jt: 0,
                        jf: 0,
                        k: 0,
                    },
                    libc::sock_filter {
                        code: (libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K) as u16,
                        jt: 0,
                        jf: 1,
                        k: denied as u32,
                    },
                    libc::sock_filter {
                        code: (libc::BPF_RET | libc::BPF_K) as u16,
                        jt: 0,
                        jf: 0,
                        k: libc::SECCOMP_RET_ERRNO | libc::EACCES as u32,
                    },
                    libc::sock_filter {
                        code: (libc::BPF_RET | libc::BPF_K) as u16,
                        jt: 0,
                        jf: 0,
                        k: libc::SECCOMP_RET_ALLOW,
                    },
                ];
                let program = libc::sock_fprog {
                    len: filters.len() as u16,
                    filter: filters.as_ptr().cast_mut(),
                };
                if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0
                    || libc::syscall(
                        libc::SYS_seccomp,
                        libc::SECCOMP_SET_MODE_FILTER,
                        0,
                        &program,
                    ) != 0
                {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::write(fd, b"1".as_ptr().cast(), 1) != 1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let error = match crate::child_owner::spawn(&mut command) {
            Err(error) => error,
            Ok((mut child, _owner)) => {
                child.wait().unwrap();
                panic!("helper escaped its ownership requirements");
            }
        };
        let mut marker = [0u8];
        assert_eq!(
            unsafe { libc::read(reader.as_raw_fd(), marker.as_mut_ptr().cast(), 1) },
            1
        );
        assert_eq!(marker, *b"1");
        assert_eq!(
            error,
            std::io::Error::from_raw_os_error(expected).to_string()
        );
    }
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
fn pinned_native_runtime_matches_the_generated_abi_contract() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/native");
    unsafe {
        let _library = crate::native::load_runtime(&path)
            .expect("prepare the pinned native runtime with scripts/prepare-native.py");
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

#[test]
fn invalid_worker_text_is_reported_and_a_later_retry_can_save_a_transcript() {
    let home = Home::new();
    let service = fake_service(&home, "invalid-utf8");
    assert!(service.preload().is_err());
    assert_eq!(service.pid.load(Ordering::SeqCst), 0);
    assert_eq!(service.snapshot()["state"]["ready"], false);
    std::fs::write(home.0.join("models/whisper-ft-test/scenario"), "").unwrap();
    let path = home.0.join("speech.wav");
    audio::save(&path, &[0.25; 160]).unwrap();
    assert_eq!(service.infer_path(&path, true).unwrap()["text"], "get hub");
    assert_eq!(
        service.store.lock().unwrap().history("", "").unwrap().len(),
        1
    );
}

#[test]
fn sending_audio_to_a_worker_that_exited_after_readiness_returns_an_error_and_releases_ownership() {
    let home = Home::new();
    let service = fake_service(&home, "exit-after-ready");
    let python = home.0.join(if cfg!(windows) {
        "environments/whisper/Scripts/python.exe"
    } else {
        "environments/whisper/bin/python"
    });
    let pid = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let mut worker = crate::worker::Worker::start(
        &service.native,
        &home.0.join("models/whisper-ft-test"),
        "cpu",
        2,
        "fixture".into(),
        pid.clone(),
        Some(&python),
        &service.config.lock().unwrap().clone(),
    )
    .unwrap();
    wait_until(|| !worker.alive());
    let path = home.0.join("speech.wav");
    audio::save(&path, &[0.25; 160]).unwrap();
    assert!(worker
        .run(&path, "en", "", std::time::Duration::from_secs(1))
        .is_err());
    drop(worker);
    assert_eq!(pid.load(Ordering::SeqCst), 0);
}

#[test]
fn failed_retained_audio_save_is_retryable_without_leaking_files_or_learning() {
    let home = Home::new();
    let mut store = home.store();
    let id = store.record(&[0.1; 160], "raw", "original", false).unwrap();
    store.db.execute_batch("CREATE TRIGGER refuse_audio BEFORE UPDATE OF audio ON samples BEGIN SELECT RAISE(ABORT, 'storage unavailable'); END;").unwrap();
    assert!(store.edit(id, "corrected").is_err());
    assert_eq!(std::fs::read_dir(home.0.join("audio")).unwrap().count(), 0);
    assert!(store.history("", "").unwrap()[0]["corrected"].is_null());
    assert!(store.learning().unwrap()["rules"]
        .as_array()
        .unwrap()
        .is_empty());
    store.db.execute_batch("DROP TRIGGER refuse_audio").unwrap();
    store.edit(id, "corrected").unwrap();
    assert!(store.audio_path(id).unwrap().is_file());
    assert_eq!(store.history("", "").unwrap()[0]["corrected"], "corrected");
}

#[test]
fn most_frequent_correction_wins_when_the_same_phrase_has_multiple_fixes() {
    let home = Home::new();
    let mut store = home.store();
    for correction in ["GitHub", "GitLab", "GitHub"] {
        let id = store.record(&[], "get hub", "get hub", false).unwrap();
        store.edit(id, correction).unwrap();
    }
    assert_eq!(store.apply("get hub", 1).unwrap(), "GitHub");
    assert_eq!(store.prompt().unwrap(), "GitHub, GitLab");
    let other = Home::new();
    let mut nonempty = other.store();
    nonempty.record(&[], "keep", "keep", false).unwrap();
    assert!(nonempty
        .import(&home.0)
        .unwrap_err()
        .contains("empty candidate history"));
    assert_eq!(nonempty.history("", "").unwrap()[0]["pasted"], "keep");
}

#[test]
fn learned_glossary_budget_includes_separators_between_terms() {
    let home = Home::new();
    let mut store = home.store();
    let mut terms = vec![];
    for index in 0..3 {
        let term = format!("{index:03}{}", "x".repeat(146));
        let id = store.record(&[], "misheard", "misheard", false).unwrap();
        store.edit(id, &term).unwrap();
        terms.push(term);
    }
    assert_eq!(store.prompt().unwrap(), terms[..2].join(", "));
    // The third term would make the native glossary exceed its 450-byte budget.
    assert!(store.prompt().unwrap().len() <= 450);
}

#[test]
fn resampling_preserves_constant_audio_at_edges_and_extreme_valid_sample_rates() {
    for rate in [1, 8000, 11025, 16000, 22050, 44100, 48000, 96000, u32::MAX] {
        let count = if rate == u32::MAX {
            268_436
        } else if rate == 1 {
            1
        } else {
            31
        };
        let audio = audio::resample(&vec![0.25; count], rate, 1);
        assert_eq!(audio.len(), (count as u64 * 16000 / rate as u64) as usize);
        assert!(!audio.is_empty());
        assert!(
            audio.iter().all(|value| (*value - 0.25).abs() < 0.00001),
            "invalid samples at {rate} Hz"
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn running_out_of_space_while_saving_settings_keeps_the_last_saved_configuration() {
    let home = Home::new();
    let before = config::defaults();
    config::save(&home.0, &before).unwrap();
    let mut changed = before.clone();
    changed["theme"] = json!("dark");
    std::os::unix::fs::symlink("/dev/full", home.0.join("config.tmp")).unwrap();
    assert!(config::save(&home.0, &changed).is_err());
    assert_eq!(config::load(&home.0).unwrap(), before);
}

#[test]
fn export_skips_missing_recordings_and_delete_still_removes_the_history_entry() {
    let home = Home::new();
    let mut store = home.store();
    let id = store.record(&[0.1; 160], "hello", "hello", true).unwrap();
    store.edit(id, "a corrected label").unwrap();
    std::fs::remove_file(store.audio_path(id).unwrap()).unwrap();
    let exported = store.export(0.7).unwrap();
    assert_eq!(
        std::fs::read_to_string(exported.join("metadata.csv")).unwrap(),
        "file_name,transcription,source\n"
    );
    store.delete(id).unwrap();
    assert!(store.history("", "").unwrap().is_empty());
}

#[test]
fn settings_with_an_unserializable_extra_value_are_rejected_without_losing_valid_settings() {
    let home = Home::new();
    let service = home.service();
    let before = service.config.lock().unwrap().clone();
    let mut changed = before.clone();
    changed["extra"] = Value::Null;
    assert!(service.update(changed).is_err());
    assert_eq!(*service.config.lock().unwrap(), before);
    assert_eq!(config::load(&home.0).unwrap(), before);
}
