use super::*;
use crate::behavior_tests::Home;

#[cfg(feature = "behavioral-tests")]
#[test]
fn ipc_commands_parse_requests_return_saved_settings_and_report_background_failures() {
    let home = Home::new();
    let service = home.service();
    let app = tauri::test::mock_builder()
        .manage(service.clone())
        .invoke_handler(tauri::generate_handler![request])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    let invoke = |body| {
        tauri::test::get_ipc_response(
            &webview,
            tauri::webview::InvokeRequest {
                cmd: "request".into(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: if cfg!(windows) {
                    "http://tauri.localhost"
                } else {
                    "tauri://localhost"
                }
                .parse()
                .unwrap(),
                body: tauri::ipc::InvokeBody::Json(body),
                headers: Default::default(),
                invoke_key: tauri::test::INVOKE_KEY.into(),
            },
        )
        .map(|response| response.deserialize::<Value>().unwrap())
    };
    assert!(invoke(json!({"args":{}})).is_err());
    assert!(invoke(json!({"op":"snapshot"})).is_err());
    assert!(invoke(json!({"op":"unknown","args":{}})).is_err());
    let mut cfg = invoke(json!({"op":"snapshot","args":{}})).unwrap()["config"].clone();
    cfg["theme"] = json!("dark");
    assert_eq!(invoke(json!({"op":"settings","args":cfg})).unwrap(), true);
    assert_eq!(crate::config::load(&home.0).unwrap()["theme"], "dark");
    assert_eq!(
        invoke(json!({"op":"snapshot","args":{}})).unwrap()["config"]["theme"],
        "dark"
    );
    let broken = service.clone();
    assert!(std::thread::spawn(move || {
        let _state = broken.state.lock().unwrap();
        panic!("Simulated background service failure");
    })
    .join()
    .is_err());
    assert!(invoke(json!({"op":"snapshot","args":{}})).is_err());
}

#[test]
fn history_commands_save_correct_revert_exclude_export_and_delete() {
    let home = Home::new();
    let service = home.service();
    let sample = service
        .store
        .lock()
        .unwrap()
        .record(&[0.1; 160], "raw", "get hub", true)
        .unwrap();
    assert_eq!(
        dispatch(&service, "snapshot", &json!({})).unwrap()["state"]["stage"],
        "Idle"
    );
    assert_eq!(
        dispatch(
            &service,
            "history",
            &json!({"search":"get hub","filter":"unedited"})
        )
        .unwrap()[0]["id"],
        sample
    );
    dispatch(
        &service,
        "correct",
        &json!({"id":sample,"text":"GitHub","replace":false}),
    )
    .unwrap();
    let learning = dispatch(&service, "learning", &json!({})).unwrap();
    assert_eq!(learning["rules"][0]["right"], "GitHub");
    dispatch(&service, "forget", &json!({"kind":"rule","key":"get hub"})).unwrap();
    dispatch(&service, "forget", &json!({"kind":"vocab","key":"GitHub"})).unwrap();
    assert_eq!(
        dispatch(&service, "learning", &json!({})).unwrap()["rules"],
        json!([])
    );
    let output = dispatch(&service, "export", &json!({})).unwrap();
    let metadata =
        std::fs::read_to_string(Path::new(output.as_str().unwrap()).join("metadata.csv")).unwrap();
    assert!(metadata.contains("GitHub"));
    assert!(!metadata.contains("get hub"));
    dispatch(&service, "exclude", &json!({"id":sample,"excluded":true})).unwrap();
    assert_eq!(
        dispatch(&service, "history", &json!({"filter":"excluded"}))
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    dispatch(&service, "revert", &json!({"id":sample})).unwrap();
    assert!(dispatch(&service, "history", &json!({"filter":"edited"}))
        .unwrap()
        .as_array()
        .unwrap()
        .is_empty());
    use base64::Engine;
    let encoded = dispatch(&service, "audio", &json!({"id":sample})).unwrap();
    let wav = base64::engine::general_purpose::STANDARD
        .decode(encoded.as_str().unwrap())
        .unwrap();
    assert_eq!(&wav[..4], b"RIFF");
    dispatch(&service, "delete", &json!({"id":sample})).unwrap();
    assert!(dispatch(&service, "history", &json!({}))
        .unwrap()
        .as_array()
        .unwrap()
        .is_empty());
    assert!(dispatch(&service, "audio", &json!({"id":sample})).is_err());
}

#[test]
fn partial_config_import_validates_before_changing_running_or_persisted_settings() {
    let home = Home::new();
    let service = home.service();
    let before = dispatch(&service, "export_config", &json!({})).unwrap();
    assert!(before.as_str().unwrap().contains("keep_mic_open = false"));
    dispatch(
        &service,
        "import_config",
        &json!({"text":"theme = 'dark'\nkeep_mic_open = true\n"}),
    )
    .unwrap();
    let cfg = dispatch(&service, "snapshot", &json!({})).unwrap()["config"].clone();
    assert_eq!(cfg["theme"], "dark");
    assert_eq!(cfg["keep_mic_open"], true);
    assert_eq!(cfg["beam_size"], 5);
    for bad in ["theme = 'invalid'", "beam_size = 'many'", "broken = ["] {
        assert!(dispatch(&service, "import_config", &json!({"text":bad})).is_err());
        assert_eq!(crate::config::load(&home.0).unwrap(), cfg);
    }
    let mut changed = cfg;
    changed["theme"] = json!("light");
    dispatch(&service, "settings", &changed).unwrap();
    assert_eq!(crate::config::load(&home.0).unwrap()["theme"], "light");
}

#[test]
fn import_copies_candidate_history_without_modifying_the_source() {
    let source = Home::new();
    let original = source.service();
    let id = original
        .store
        .lock()
        .unwrap()
        .record(&[0.1; 160], "raw", "words", true)
        .unwrap();
    let home = Home::new();
    let service = home.service();
    assert_eq!(
        dispatch(&service, "import", &json!({"path":source.0})).unwrap(),
        1
    );
    assert_eq!(
        dispatch(&service, "history", &json!({})).unwrap()[0]["id"],
        id
    );
    dispatch(&service, "delete", &json!({"id":id})).unwrap();
    assert_eq!(
        original
            .store
            .lock()
            .unwrap()
            .history("", "")
            .unwrap()
            .len(),
        1
    );
    assert!(original
        .store
        .lock()
        .unwrap()
        .audio_path(id)
        .unwrap()
        .exists());
}

#[test]
fn malformed_commands_are_refused_without_changing_history_or_starting_work() {
    let home = Home::new();
    let service = home.service();
    for (command, args) in [
        ("unknown", json!({})),
        ("correct", json!({})),
        ("correct", json!({"id":1})),
        ("audio", json!({"id":"not a number"})),
        ("copy", json!({})),
        ("import", json!({})),
        ("import_config", json!({})),
        ("install", json!({})),
        ("install", json!({"id":"unknown"})),
        ("reuse_model", json!({})),
        ("reuse_model", json!({"id":"moonshine-base"})),
        (
            "reuse_model",
            json!({"id":"kyutai-stt-1b","path":"missing"}),
        ),
        (
            "reuse_model",
            json!({"id":"moonshine-base","path":"missing"}),
        ),
        ("load", json!({})),
        ("train", json!({})),
        ("rollback", json!({})),
    ] {
        assert!(
            dispatch(&service, command, &args).is_err(),
            "accepted {command}: {args}"
        );
    }
    assert!(!service.downloading.load(Ordering::SeqCst));
    assert!(service
        .store
        .lock()
        .unwrap()
        .history("", "")
        .unwrap()
        .is_empty());
    service.learning_busy.store(true, Ordering::SeqCst);
    assert!(
        dispatch(&service, "install", &json!({"id":"moonshine-base"}))
            .unwrap_err()
            .contains("Pause learning")
    );
    service.learning_busy.store(false, Ordering::SeqCst);
    service.downloading.store(true, Ordering::SeqCst);
    assert!(
        dispatch(&service, "install", &json!({"id":"moonshine-base"}))
            .unwrap_err()
            .contains("One model")
    );
    dispatch(&service, "cancel", &json!({})).unwrap();
    assert_eq!(service.snapshot()["state"]["stage"], "Cancelled");
}

#[test]
fn optional_install_commands_report_progress_success_failure_and_cancellation() {
    use crate::behavior_tests::wait_until;
    for mode in ["success", "failure", "cancel"] {
        let home = Home::new();
        let service = home.service();
        std::fs::write(home.0.join("resources/install-exception.py"), format!(
            "import pathlib,sys,time\nr=pathlib.Path(sys.argv[sys.argv.index('--root')+1])\nprint('{{\"stage\":\"Installing fixture\"}}',flush=True)\nwhile not (r/'finish').exists():time.sleep(.01)\nraise SystemExit({})\n", if mode == "failure" { 3 } else { 0 }
        )).unwrap();
        assert_eq!(
            dispatch(&service, "install", &json!({"id":"training"})).unwrap(),
            true
        );
        wait_until(|| service.snapshot()["state"]["download"]["stage"] == "Installing fixture");
        assert!(service.downloading.load(Ordering::SeqCst));
        assert!(dispatch(
            &service,
            "settings",
            &service.config.lock().unwrap().clone()
        )
        .is_err());
        assert!(dispatch(&service, "install", &json!({"id":"training"})).is_err());
        if mode == "cancel" {
            dispatch(&service, "cancel", &json!({})).unwrap();
        } else {
            std::fs::write(home.0.join("finish"), "").unwrap();
        }
        let expected = if mode == "success" {
            "Installed"
        } else {
            "Failed"
        };
        wait_until(|| service.snapshot()["state"]["download"]["stage"] == expected);
        let status = service.snapshot()["state"]["download"].clone();
        assert_eq!(status["id"], "training");
        if mode == "cancel" {
            assert!(status["error"].as_str().unwrap().contains("cancelled"));
        }
        assert!(!service.downloading.load(Ordering::SeqCst));
        assert_eq!(service.install_pid.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn an_unverified_complete_download_never_replaces_an_existing_model() {
    use crate::behavior_tests::wait_until;
    let home = Home::new();
    let service = home.service();
    let model = crate::models::path(&home.0, "moonshine-base").unwrap();
    std::fs::write(&model, "Existing model").unwrap();
    let partial = model.with_extension("partial");
    let file = std::fs::File::create(&partial).unwrap();
    file.set_len(
        crate::models::spec("moonshine-base").unwrap()["gguf"]["size"]
            .as_u64()
            .unwrap(),
    )
    .unwrap();
    drop(file);
    dispatch(&service, "install", &json!({"id":"moonshine-base"})).unwrap();
    wait_until(|| service.snapshot()["state"]["download"]["stage"] == "Failed");
    assert!(service.snapshot()["state"]["download"]["error"]
        .as_str()
        .unwrap()
        .contains("SHA-256 mismatch"));
    assert_eq!(std::fs::read_to_string(model).unwrap(), "Existing model");
    assert!(!partial.exists());
    assert!(!service.downloading.load(Ordering::SeqCst));
}

#[test]
#[ignore = "Requires the pinned GGUF supplied by test-coverage.sh"]
fn reuse_model_validates_and_copies_the_real_artifact_without_losing_it_on_disk_errors() {
    let home = Home::new();
    let service = home.service();
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../.build/fixtures/moonshine-base.gguf");
    let destination = crate::models::path(&home.0, "moonshine-base").unwrap();
    let partial = destination.with_extension("partial");
    let args = json!({"id":"moonshine-base","path":source});
    std::fs::write(&destination, "Old installation").unwrap();
    std::fs::write(&partial, "Interrupted replacement").unwrap();
    assert_eq!(dispatch(&service, "reuse_model", &args).unwrap(), true);
    let expected = std::fs::read(&source).unwrap();
    assert!(std::fs::read(&destination).unwrap() == expected);
    assert!(!partial.exists());
    assert_eq!(
        service.snapshot()["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|model| model["id"] == "moonshine-base")
            .unwrap()["installed"],
        true
    );
    std::fs::create_dir(&partial).unwrap();
    assert!(dispatch(&service, "reuse_model", &args).is_err());
    assert!(std::fs::read(&destination).unwrap() == expected);
    std::fs::remove_dir(&partial).unwrap();
    std::fs::remove_file(&destination).unwrap();
    std::fs::create_dir(&destination).unwrap();
    assert!(dispatch(&service, "reuse_model", &args).is_err());
    assert!(destination.is_dir());
    assert!(std::fs::read(&source).unwrap() == expected);
}

#[test]
fn load_and_train_commands_keep_the_current_model_when_training_is_refused_or_rejected() {
    use crate::behavior_tests::{fake_service, training_environment, wait_until};
    let home = Home::new();
    let service = fake_service(&home, "");
    assert_eq!(dispatch(&service, "load", &json!({})).unwrap(), true);
    assert_eq!(service.snapshot()["state"]["ready"], true);
    training_environment(
        &home,
        r#"{"status":"rejected","gate1":{"passed":false},"gate2":{"passed":true}}"#,
    );
    let before = service.config.lock().unwrap().clone();
    if let Err(error) = dispatch(&service, "train", &json!({})) {
        assert!(
            error.contains("battery"),
            "unexpected training refusal: {error}"
        );
        assert!(!service.learning_busy.load(Ordering::SeqCst));
        assert_eq!(crate::config::load(&home.0).unwrap(), before);
        return;
    }
    wait_until(|| !service.learning_busy.load(Ordering::SeqCst));
    assert_eq!(
        service
            .store
            .lock()
            .unwrap()
            .get_state("training_status")
            .unwrap()["status"],
        "rejected"
    );
    assert_eq!(crate::config::load(&home.0).unwrap(), before);
}

#[test]
fn manual_recording_refuses_unavailable_or_busy_recorder_and_recovers() {
    let home = Home::new();
    let service = home.service();
    assert!(dispatch(&service, "manual_stop", &json!({}))
        .unwrap_err()
        .contains("No app recording"));
    assert!(dispatch(&service, "manual_start", &json!({}))
        .unwrap_err()
        .contains("unavailable"));
    assert!(!service.recording.load(Ordering::SeqCst));
    let (tx, rx) = mpsc::sync_channel(1);
    *service.capture_tx.lock().unwrap() = Some(tx);
    service.learning_busy.store(true, Ordering::SeqCst);
    // Preempting learning sends a cancellation; a full recorder queue refuses
    // the following start instead of claiming the microphone is recording.
    assert!(dispatch(&service, "manual_start", &json!({}))
        .unwrap_err()
        .contains("busy"));
    assert_eq!(rx.recv().unwrap().0, Action::Cancel);
    assert!(!service.recording.load(Ordering::SeqCst));
    service.learning_busy.store(false, Ordering::SeqCst);
    dispatch(&service, "manual_start", &json!({})).unwrap();
    assert_eq!(rx.recv().unwrap().0, Action::Start);
    assert!(service.snapshot()["manual_recording"].as_bool().unwrap());
    assert!(dispatch(&service, "manual_start", &json!({}))
        .unwrap_err()
        .contains("already active"));
    dispatch(&service, "manual_stop", &json!({})).unwrap();
    assert_eq!(rx.recv().unwrap().0, Action::Cancel);
    service.config.lock().unwrap()["min_hold_ms"] = json!(0);
    dispatch(&service, "manual_start", &json!({})).unwrap();
    assert_eq!(rx.recv().unwrap().0, Action::Start);
    dispatch(&service, "manual_stop", &json!({})).unwrap();
    assert_eq!(rx.recv().unwrap().0, Action::Submit);
    dispatch(&service, "manual_start", &json!({})).unwrap();
    assert!(dispatch(&service, "manual_stop", &json!({}))
        .unwrap_err()
        .contains("busy"));
    assert!(!service.recording.load(Ordering::SeqCst));
}

#[cfg(target_os = "linux")]
struct Running {
    service: Arc<Service>,
    threads: Vec<std::thread::JoinHandle<()>>,
    visibility: Arc<std::sync::Mutex<Vec<bool>>>,
    navigation: Arc<std::sync::Mutex<Vec<String>>>,
}
#[cfg(target_os = "linux")]
impl Running {
    fn new(service: Arc<Service>) -> Self {
        let visibility = Arc::new(std::sync::Mutex::new(Vec::new()));
        let changes = visibility.clone();
        let navigation = Arc::new(std::sync::Mutex::new(Vec::new()));
        let pages = navigation.clone();
        let threads = start_runtime(
            service.clone(),
            move |page| pages.lock().unwrap().push(page.to_owned()),
            move |visible| {
                changes.lock().unwrap().push(visible);
            },
        )
        .into_iter()
        .collect();
        Self {
            service,
            threads,
            visibility,
            navigation,
        }
    }
    fn start(&self) {
        dispatch(&self.service, "manual_start", &json!({})).unwrap();
        let started = Instant::now();
        while self.visibility.lock().unwrap().last() != Some(&true)
            && started.elapsed() < Duration::from_secs(10)
        {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            self.visibility.lock().unwrap().last().copied(),
            Some(true),
            "{}; config {}",
            self.service.snapshot()["state"],
            self.service.config.lock().unwrap()
        );
    }
}
#[cfg(target_os = "linux")]
impl Drop for Running {
    fn drop(&mut self) {
        self.service.shutdown();
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "Uses the synthetic ALSA input and isolated Xvfb supplied by test-coverage.sh"]
fn app_recording_submits_audio_to_history_and_hides_the_finished_indicator() {
    use crate::behavior_tests::{fake_service, wait_until};
    let _desktop = crate::behavior_tests::DESKTOP
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    assert_eq!(
        std::env::var("SPEECHIFY_TEST_SYNTHETIC_ALSA").as_deref(),
        Ok("1")
    );
    for keep in [false, true] {
        let home = Home::new();
        let service = fake_service(&home, "");
        let mut cfg = service.config.lock().unwrap().clone();
        cfg["min_hold_ms"] = json!(1);
        cfg["keep_mic_open"] = json!(keep);
        service.update(cfg).unwrap();
        let runtime = Running::new(service.clone());
        runtime.start();
        std::thread::sleep(Duration::from_millis(20));
        dispatch(&service, "manual_stop", &json!({})).unwrap();
        wait_until(|| service.snapshot()["state"]["recent"] == "get hub");
        let history = dispatch(&service, "history", &json!({})).unwrap();
        assert_eq!(history.as_array().unwrap().len(), 1);
        assert_eq!(history[0]["raw"], "get hub");
        assert!(service.snapshot()["state"]["error"]
            .as_str()
            .unwrap()
            .contains("Use Copy text"));
        wait_until(|| runtime.visibility.lock().unwrap().last() == Some(&false));
        assert!(!std::fs::read_dir(&home.0).unwrap().any(|p| p
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("pending-")));
        assert!(!service.snapshot()["manual_recording"].as_bool().unwrap());
    }
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "Uses the synthetic ALSA input and isolated Xvfb supplied by test-coverage.sh"]
fn cancelling_an_app_recording_clears_audio_without_saving_history_and_allows_another_hold() {
    use crate::behavior_tests::{fake_service, wait_until};
    let _desktop = crate::behavior_tests::DESKTOP
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    assert_eq!(
        std::env::var("SPEECHIFY_TEST_SYNTHETIC_ALSA").as_deref(),
        Ok("1")
    );
    for keep in [false, true] {
        let home = Home::new();
        let service = fake_service(&home, "");
        service.config.lock().unwrap()["keep_mic_open"] = json!(keep);
        let runtime = Running::new(service.clone());
        runtime.start();
        // A deliberately long minimum makes this release a cancellation.
        service.config.lock().unwrap()["min_hold_ms"] = json!(60_000);
        dispatch(&service, "manual_stop", &json!({})).unwrap();
        wait_until(|| runtime.visibility.lock().unwrap().last() == Some(&false));
        assert!(dispatch(&service, "history", &json!({}))
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(service.snapshot()["state"]["stage"], "Ready");
        runtime.start();
        dispatch(&service, "cancel", &json!({})).unwrap();
        wait_until(|| runtime.visibility.lock().unwrap().last() == Some(&false));
        assert!(dispatch(&service, "history", &json!({}))
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty());
    }
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "Uses the synthetic ALSA input and isolated Xvfb supplied by test-coverage.sh"]
fn global_holds_cancel_on_escape_reload_shortcuts_and_open_the_requested_pages() {
    use crate::behavior_tests::{fake_service, wait_until};
    let _desktop = crate::behavior_tests::DESKTOP
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    assert_eq!(
        std::env::var("SPEECHIFY_TEST_SYNTHETIC_ALSA").as_deref(),
        Ok("1")
    );
    let home = Home::new();
    let service = fake_service(&home, "");
    let mut cfg = service.config.lock().unwrap().clone();
    cfg["min_hold_ms"] = json!(1);
    service.update(cfg.clone()).unwrap();
    let runtime = Running::new(service.clone());
    let key = |action: &str, name: &str| {
        // Xvfb's physical right modifier codes avoid xdotool's keysym
        // synthesis, which also presses the corresponding left modifier.
        let name = match name {
            "Control_R" => "105",
            "Alt_R" => "108",
            other => other,
        };
        assert!(std::process::Command::new("xdotool")
            .args([action, name])
            .status()
            .unwrap()
            .success());
    };
    // Leave enough time for the native poller to open its X11 connection.
    std::thread::sleep(Duration::from_millis(100));
    for (shortcut, page) in [("f", "History"), ("h", "History"), ("m", "Models")] {
        let before = runtime.navigation.lock().unwrap().len();
        key("keydown", "Control_L");
        key("keydown", "Alt_L");
        key("keydown", shortcut);
        wait_until(|| runtime.navigation.lock().unwrap().len() == before + 1);
        key("keyup", shortcut);
        key("keyup", "Alt_L");
        key("keyup", "Control_L");
        assert_eq!(runtime.navigation.lock().unwrap().last().unwrap(), page);
        std::thread::sleep(Duration::from_millis(20));
    }
    key("keydown", "Control_R");
    wait_until(|| service.snapshot()["state"]["stage"] == "Recording");
    key("keydown", "Escape");
    wait_until(|| runtime.visibility.lock().unwrap().last() == Some(&false));
    key("keyup", "Escape");
    key("keyup", "Control_R");
    assert!(dispatch(&service, "history", &json!({}))
        .unwrap()
        .as_array()
        .unwrap()
        .is_empty());
    cfg["ptt_key"] = json!("alt_r");
    dispatch(&service, "settings", &cfg).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    service.learning_busy.store(true, Ordering::SeqCst);
    key("keydown", "Alt_R");
    wait_until(|| service.snapshot()["state"]["stage"] == "Recording");
    key("keyup", "Alt_R");
    wait_until(|| {
        dispatch(&service, "history", &json!({}))
            .unwrap()
            .as_array()
            .unwrap()
            .len()
            == 1
    });
    assert_eq!(service.snapshot()["state"]["recent"], "get hub");
    service.learning_busy.store(false, Ordering::SeqCst);
    runtime.start();
    key("keydown", "Alt_R");
    key("keyup", "Alt_R");
    // Global shortcut events cannot end a recording started from the app.
    assert!(service.snapshot()["manual_recording"].as_bool().unwrap());
    dispatch(&service, "cancel", &json!({})).unwrap();
}

#[cfg(not(windows))]
#[test]
fn a_correction_is_saved_when_the_platform_cannot_replace_the_original_target() {
    let home = Home::new();
    let service = home.service();
    let sample = service
        .store
        .lock()
        .unwrap()
        .record(&[0.1; 160], "raw", "get hub", true)
        .unwrap();
    assert!(crate::replacement::capture(1, "get hub").is_none());
    *service.replacement.lock().unwrap() = Some((
        sample,
        crate::replacement::Receipt {
            target: 1,
            expected: vec![],
            start: 0,
            end: 0,
            process: 0,
            created: Instant::now(),
        },
    ));
    let error = dispatch(
        &service,
        "correct",
        &json!({"id":sample,"text":"GitHub","replace":true}),
    )
    .unwrap_err();
    assert!(error.contains("Saved correction only"));
    assert_eq!(
        dispatch(&service, "history", &json!({})).unwrap()[0]["corrected"],
        "GitHub"
    );
    assert!(service.replacement.lock().unwrap().is_none());
}

#[cfg(target_os = "linux")]
#[test]
fn login_startup_respects_xdg_and_home_and_reports_filesystem_failures() {
    let root = match std::env::var_os("SPEECHIFY_TEST_AUTOSTART_ROOT") {
        Some(path) => std::path::PathBuf::from(path),
        None => {
            let home = Home::new();
            for mode in ["xdg", "home", "xdg_without_home", "missing"] {
                let root = home.0.join(mode);
                std::fs::create_dir(&root).unwrap();
                let mut child = std::process::Command::new(std::env::current_exe().unwrap());
                child.args(["--exact", "desktop::behavior::login_startup_respects_xdg_and_home_and_reports_filesystem_failures"])
                    .env("SPEECHIFY_TEST_AUTOSTART_ROOT", &root)
                    .env_remove("HOME").env_remove("XDG_CONFIG_HOME");
                if ["xdg", "xdg_without_home"].contains(&mode) {
                    child.env("XDG_CONFIG_HOME", &root);
                }
                if ["home", "xdg"].contains(&mode) {
                    child.env("HOME", &root);
                }
                let output = child.output().unwrap();
                assert!(
                    output.status.success(),
                    "{mode}: {}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            return;
        }
    };
    let home = Home::new();
    let service = home.service();
    let toggle = |enabled| dispatch(&service, "autostart", &json!({"enabled":enabled}));
    if std::env::var_os("HOME").is_none() && std::env::var_os("XDG_CONFIG_HOME").is_none() {
        assert!(toggle(true).is_err());
        assert!(std::fs::read_dir(root).unwrap().next().is_none());
        return;
    }
    let base = if std::env::var_os("XDG_CONFIG_HOME").is_some() {
        root
    } else {
        root.join(".config")
    };
    let parent = base.join("autostart");
    let entry = parent.join("speechify-tauri-candidate.desktop");
    toggle(false).unwrap();
    toggle(true).unwrap();
    let first = std::fs::read_to_string(&entry).unwrap();
    assert!(first.contains("Name=Speechify Tauri Candidate\n"));
    assert!(first.contains(&format!(
        "Exec=\"{}\"\n",
        std::env::current_exe().unwrap().display()
    )));
    toggle(true).unwrap();
    assert_eq!(std::fs::read_to_string(&entry).unwrap(), first);
    toggle(false).unwrap();
    toggle(false).unwrap();
    assert!(!entry.exists());
    std::fs::remove_dir(&parent).unwrap();
    std::fs::write(&parent, "unrelated user configuration").unwrap();
    assert!(toggle(true).is_err());
    assert_eq!(
        std::fs::read_to_string(&parent).unwrap(),
        "unrelated user configuration"
    );
    std::fs::remove_file(&parent).unwrap();
    std::fs::create_dir_all(&entry).unwrap();
    assert!(toggle(true).is_err());
    assert!(toggle(false).is_err());
    assert!(entry.is_dir());
}

#[cfg(not(target_os = "linux"))]
#[test]
#[ignore = "Changes candidate login startup; requires a disposable native desktop"]
fn login_startup_can_be_enabled_verified_and_disabled_on_native_desktops() {
    assert_eq!(
        std::env::var("SPEECHIFY_TEST_DISPOSABLE_DESKTOP").as_deref(),
        Ok("1")
    );
    let home = Home::new();
    if std::env::var_os("SPEECHIFY_TEST_LOGIN_CHILD").is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "desktop::behavior::login_startup_can_be_enabled_verified_and_disabled_on_native_desktops", "--include-ignored"])
            .env("SPEECHIFY_TEST_LOGIN_CHILD", "1").env("HOME", &home.0).output().unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let service = home.service();
    dispatch(&service, "autostart", &json!({"enabled": true})).unwrap();
    #[cfg(target_os = "macos")]
    {
        let path = std::path::PathBuf::from(std::env::var_os("HOME").unwrap())
            .join("Library/LaunchAgents/org.sumedhbhat.speechify.tauri.plist");
        let check = std::process::Command::new("/usr/bin/plutil")
            .args(["-extract", "ProgramArguments.0", "raw", "-o", "-"])
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            check.status.success(),
            "{}",
            String::from_utf8_lossy(&check.stderr)
        );
        assert_eq!(
            String::from_utf8(check.stdout).unwrap().trim(),
            std::env::current_exe().unwrap().to_str().unwrap()
        );
        dispatch(&service, "autostart", &json!({"enabled": false})).unwrap();
        assert!(!path.exists());
        std::fs::create_dir(&path).unwrap();
        assert!(dispatch(&service, "autostart", &json!({"enabled": false})).is_err());
        std::fs::remove_dir(&path).unwrap();
        std::fs::write(&path, "prior entry").unwrap();
        assert!(dispatch(&service, "autostart", &json!({"enabled": true})).is_ok());
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(path.parent().unwrap()).unwrap();
        std::fs::write(path.parent().unwrap(), "obstructed directory").unwrap();
        assert!(dispatch(&service, "autostart", &json!({"enabled": true})).is_err());
        assert_eq!(
            std::fs::read_to_string(path.parent().unwrap()).unwrap(),
            "obstructed directory"
        );
    }
    #[cfg(windows)]
    {
        let result = std::process::Command::new("reg")
            .args([
                "query",
                "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run",
                "/v",
                "SpeechifyTauriCandidate",
            ])
            .output()
            .unwrap();
        assert!(result.status.success());
        let entry = String::from_utf8_lossy(&result.stdout);
        assert!(entry.contains(&format!(
            "\"{}\"",
            std::env::current_exe().unwrap().display()
        )));
        dispatch(&service, "autostart", &json!({"enabled": false})).unwrap();
        assert!(dispatch(&service, "autostart", &json!({"enabled": false})).is_err());
    }
}

#[cfg(target_os = "macos")]
#[test]
fn unsupported_macos_shortcuts_are_refused_without_changing_saved_settings() {
    let home = Home::new();
    let service = home.service();
    let before = service.config.lock().unwrap().clone();
    for key in ["scroll_lock", "f21", "f22", "f23", "f24"] {
        let mut cfg = before.clone();
        cfg["ptt_key"] = json!(key);
        assert!(dispatch(&service, "settings", &cfg).is_err());
        assert_eq!(*service.config.lock().unwrap(), before);
        assert_eq!(crate::config::load(&home.0).unwrap(), before);
    }
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "Requires the synthetic ALSA input and isolated Xvfb"]
fn microphone_open_and_disconnect_failures_are_visible_and_never_save_partial_dictations() {
    use crate::behavior_tests::{fake_service, wait_until};
    let _desktop = crate::behavior_tests::DESKTOP
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    for (input, keep) in [
        ("Speechify float setup error", false),
        ("Speechify float setup error", true),
        ("Speechify float disconnect", true),
    ] {
        let home = Home::new();
        let service = fake_service(&home, "");
        let mut cfg = service.config.lock().unwrap().clone();
        cfg["input_device"] = json!(input);
        cfg["min_hold_ms"] = json!(1);
        cfg["keep_mic_open"] = json!(keep);
        service.update(cfg).unwrap();
        let runtime = Running::new(service.clone());
        if !keep {
            dispatch(&service, "manual_start", &json!({})).unwrap();
        }
        wait_until(|| service.snapshot()["state"]["stage"] == "Error");
        assert!(!service.recording.load(Ordering::SeqCst));
        assert!(dispatch(&service, "history", &json!({}))
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty());
        if !keep {
            dispatch(&service, "manual_stop", &json!({})).unwrap();
            service
                .capture_tx
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .send((Action::Submit, 0, service.generation.load(Ordering::SeqCst)))
                .unwrap();
            service
                .capture_tx
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .send((Action::Cancel, 0, service.generation.load(Ordering::SeqCst)))
                .unwrap();
            wait_until(|| runtime.visibility.lock().unwrap().last() == Some(&false));
        }
        let mut cfg = service.config.lock().unwrap().clone();
        cfg["input_device"] = json!("Speechify float input");
        service.update(cfg).unwrap();
        if keep {
            wait_until(|| service.snapshot()["state"]["actual_input"] == "Speechify float input");
        }
        runtime.start();
        wait_until(|| service.snapshot()["state"]["level"].as_f64().unwrap_or(0.) > 0.);
        std::thread::sleep(Duration::from_millis(100));
        dispatch(&service, "manual_stop", &json!({})).unwrap();
        wait_until(|| service.snapshot()["state"]["recent"] == "get hub");
        assert_eq!(
            dispatch(&service, "history", &json!({}))
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "Requires the synthetic ALSA input and isolated Xvfb"]
fn a_full_dictation_queue_reports_backpressure_and_cancel_or_shutdown_discards_old_clips() {
    use crate::behavior_tests::{fake_service, wait_until};
    let _desktop = crate::behavior_tests::DESKTOP
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    for shutdown in [false, true] {
        let home = Home::new();
        let service = fake_service(&home, "slow");
        let mut cfg = service.config.lock().unwrap().clone();
        cfg["input_device"] = json!("Speechify float input");
        cfg["min_hold_ms"] = json!(1);
        service.update(cfg).unwrap();
        let runtime = Running::new(service.clone());
        let record = || {
            dispatch(&service, "manual_start", &json!({})).unwrap();
            wait_until(|| service.snapshot()["state"]["stage"] == "Recording");
            wait_until(|| service.snapshot()["state"]["level"].as_f64().unwrap_or(0.) > 0.);
            std::thread::sleep(Duration::from_millis(100));
            dispatch(&service, "manual_stop", &json!({})).unwrap();
        };
        record();
        wait_until(|| {
            home.0.join("models/whisper-ft-test/request.json").exists()
                || service.snapshot()["state"]["stage"] == "Error"
        });
        assert!(
            home.0.join("models/whisper-ft-test/request.json").exists(),
            "{}",
            service.snapshot()["state"]
        );
        for index in 0..5 {
            record();
            if index < 4 {
                wait_until(|| {
                    ["Queued", "Error"]
                        .iter()
                        .any(|stage| service.snapshot()["state"]["stage"] == *stage)
                });
                assert_eq!(
                    service.snapshot()["state"]["stage"],
                    "Queued",
                    "clip {index}: {}",
                    service.snapshot()["state"]
                );
            } else {
                wait_until(|| {
                    service.snapshot()["state"]["error"]
                        .as_str()
                        .is_some_and(|error| error.contains("queue is full"))
                });
            }
        }
        assert!(service
            .store
            .lock()
            .unwrap()
            .history("", "")
            .unwrap()
            .is_empty());
        if shutdown {
            service.shutdown();
        } else {
            dispatch(&service, "cancel", &json!({})).unwrap();
            std::fs::write(home.0.join("models/whisper-ft-test/scenario"), "").unwrap();
            record();
            wait_until(|| service.snapshot()["state"]["recent"] == "get hub");
            assert_eq!(
                service.store.lock().unwrap().history("", "").unwrap().len(),
                1
            );
        }
        drop(runtime);
        assert!(!std::fs::read_dir(&home.0).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("pending-")));
    }
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "Requires CI's isolated /dev/input/by-id directory"]
fn wayland_keyboard_disconnect_cancels_recording_without_saving_partial_audio() {
    use crate::behavior_tests::{fake_service, wait_until};
    use std::io::Write;
    assert_eq!(
        std::env::var("SPEECHIFY_TEST_VIRTUAL_KEYBOARD").as_deref(),
        Ok("1")
    );
    if std::env::var_os("SPEECHIFY_TEST_WAYLAND_CHILD").is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "desktop::behavior::wayland_keyboard_disconnect_cancels_recording_without_saving_partial_audio", "--include-ignored"])
            .env("SPEECHIFY_TEST_WAYLAND_CHILD", "1").env("WAYLAND_DISPLAY", "isolated-test-session").env("XDG_SESSION_TYPE", "wayland").output().unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let path = std::path::PathBuf::from(format!(
        "/dev/input/by-id/speechify-{}-event-kbd",
        std::process::id()
    ));
    struct Fifo(std::path::PathBuf);
    impl Drop for Fifo {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _fifo = Fifo(path.clone());
    assert_eq!(
        unsafe {
            libc::mkfifo(
                std::ffi::CString::new(path.to_str().unwrap())
                    .unwrap()
                    .as_ptr(),
                0o600,
            )
        },
        0
    );
    let mut writer = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    let poller = platform::Poller::new().unwrap();
    assert!(poller.down().unwrap().is_empty());
    assert!(poller.focus().unwrap_err().contains("Wayland"));
    drop(poller);
    let home = Home::new();
    let service = fake_service(&home, "");
    let runtime = Running::new(service.clone());
    let event = libc::input_event {
        time: libc::timeval {
            tv_sec: 0,
            tv_usec: 0,
        },
        type_: 1,
        code: 97,
        value: 1,
    };
    let bytes = unsafe {
        std::slice::from_raw_parts(
            (&event as *const libc::input_event).cast::<u8>(),
            std::mem::size_of_val(&event),
        )
    };
    writer.write_all(bytes).unwrap();
    wait_until(|| runtime.visibility.lock().unwrap().last() == Some(&true));
    drop(writer);
    wait_until(|| {
        service.snapshot()["state"]["error"]
            .as_str()
            .is_some_and(|error| error.contains("Keyboard disconnected"))
    });
    assert!(!service.recording.load(Ordering::SeqCst));
    wait_until(|| runtime.visibility.lock().unwrap().last() == Some(&false));
    assert!(service
        .store
        .lock()
        .unwrap()
        .history("", "")
        .unwrap()
        .is_empty());
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "Requires the synthetic ALSA input and isolated Xvfb"]
fn failed_recognition_is_reported_without_saving_a_guess_or_leaving_temporary_audio() {
    use crate::behavior_tests::{fake_service, wait_until};
    let _desktop = crate::behavior_tests::DESKTOP
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    let service = fake_service(&home, "error");
    let mut cfg = service.config.lock().unwrap().clone();
    cfg["input_device"] = json!("Speechify float input");
    cfg["min_hold_ms"] = json!(1);
    service.update(cfg).unwrap();
    let runtime = Running::new(service.clone());
    runtime.start();
    wait_until(|| service.snapshot()["state"]["level"].as_f64().unwrap_or(0.) > 0.);
    std::thread::sleep(Duration::from_millis(100));
    dispatch(&service, "manual_stop", &json!({})).unwrap();
    wait_until(|| service.snapshot()["state"]["error"] == "bad clip");
    assert!(service
        .store
        .lock()
        .unwrap()
        .history("", "")
        .unwrap()
        .is_empty());
    assert_eq!(service.snapshot()["state"]["recent"], "");
    assert!(!std::fs::read_dir(&home.0).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with("pending-")));
}
