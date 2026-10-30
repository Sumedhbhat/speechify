use super::*;
use crate::behavior_tests::{fake_service, training_environment, wait_until, Home};

#[cfg(unix)]
#[test]
fn teacher_selection_uses_the_first_reported_gpu_and_refuses_unavailable_gpu_information() {
    use std::os::unix::fs::PermissionsExt;
    let bin = match std::env::var_os("SPEECHIFY_TEST_GPU_QUERY") {
        Some(bin) => std::path::PathBuf::from(bin),
        None => {
            let home = Home::new();
            let bin = home.0.join("tools");
            std::fs::create_dir(&bin).unwrap();
            let output = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "learning::behavior::teacher_selection_uses_the_first_reported_gpu_and_refuses_unavailable_gpu_information"])
                .env("SPEECHIFY_TEST_GPU_QUERY", &bin).env("PATH", &bin).output().unwrap();
            assert!(
                output.status.success(),
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
    };
    let query = bin.join("nvidia-smi");
    for (body, expected) in [
        ("printf '6144\\n16384\\n'", Some(6.)),
        ("printf 'not available'", None),
        ("printf '\\377'", None),
        ("exit 0", None),
        ("exit 1", None),
    ] {
        std::fs::write(&query, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&query, std::fs::Permissions::from_mode(0o700)).unwrap();
        let memory = gpu_memory();
        assert_eq!(memory, expected);
        match memory {
            Some(_) => assert_eq!(
                choose_teacher("auto", memory, false).unwrap(),
                ("qwen3-asr-1.7b".into(), "cuda")
            ),
            None => assert!(choose_teacher("auto", memory, false).is_err()),
        }
    }
    std::fs::remove_file(query).unwrap();
    assert!(gpu_memory().is_none());
    assert!(choose_teacher("auto", gpu_memory(), false).is_err());
}

#[test]
fn retraining_requires_enough_new_labels_or_elapsed_time_with_new_labels() {
    let cfg = crate::config::defaults();
    for (total, previous, days, expected) in [
        (49, 0, 100., false),
        (50, 50, 100., false),
        (50, 51, 100., false),
        (50, 49, 7., true),
        (50, 49, 6., false),
        (350, 50, 0., true),
        (349, 50, 0., false),
    ] {
        assert_eq!(
            due(total, previous, 0., days * 86400., &cfg),
            expected,
            "{total} labels, {previous} previous, {days} days"
        );
    }
}

#[test]
fn labels_reject_empty_implausibly_long_and_repeated_speech() {
    assert_eq!(judge("", 2.), (0., Some("empty".into())));
    assert_eq!(
        judge("one two three four five six seven eight nine ten", 1.),
        (0., Some("too long for clip".into()))
    );
    for text in [
        "yes yes yes yes yes",
        "thank you thank you thank you",
        "one two three one two three one two three",
        "one two three four one two three four one two three four",
    ] {
        assert_eq!(judge(text, 10.), (0., Some("repeated phrase".into())));
    }
    assert_eq!(judge("hello there", 1.), (0.8, None));
    assert!(judge("hello", 10.).0 < 0.7);
    assert!(judge("one two three four five six seven", 1.).0 < 0.7);
}

#[test]
fn training_refuses_busy_unsupported_and_uninstalled_environments_without_latching_busy() {
    let home = Home::new();
    let service = home.service();
    service.recording.store(true, Ordering::SeqCst);
    assert!(start_training(service.clone()).is_err());
    service.recording.store(false, Ordering::SeqCst);
    service.downloading.store(true, Ordering::SeqCst);
    assert!(start_training(service.clone()).is_err());
    service.downloading.store(false, Ordering::SeqCst);
    service.learning_busy.store(true, Ordering::SeqCst);
    assert!(start_training(service.clone())
        .unwrap_err()
        .contains("already running"));
    service.learning_busy.store(false, Ordering::SeqCst);
    assert!(start_training(service.clone())
        .unwrap_err()
        .contains("Select Whisper"));
    assert!(!service.learning_busy.load(Ordering::SeqCst));
    service.config.lock().unwrap()["model"] = json!("whisper-large-v3");
    assert!(start_training(service.clone())
        .unwrap_err()
        .contains("Install the optional training"));
    assert!(!service.learning_busy.load(Ordering::SeqCst));
}

// These exercise real child processes and the adoption boundary. The power
// reading is supplied at the OS boundary so every case runs on laptops too.
#[test]
fn rejected_failed_and_malformed_training_never_replace_the_current_model() {
    for report in [
        "exit",
        "invalid json",
        r#"{"status":"rejected","gate1":{"passed":true},"gate2":{"passed":false}}"#,
    ] {
        let home = Home::new();
        let service = fake_service(&home, "");
        training_environment(&home, report);
        let before = service.config.lock().unwrap().clone();
        start_training_on_power(service.clone(), true).unwrap();
        wait_until(|| !service.learning_busy.load(Ordering::SeqCst));
        assert_eq!(*service.config.lock().unwrap(), before);
        assert_eq!(crate::config::load(&home.0).unwrap(), before);
        assert!(service
            .store
            .lock()
            .unwrap()
            .get_state("last_training")
            .is_ok());
        let status = service
            .store
            .lock()
            .unwrap()
            .get_state("training_status")
            .unwrap();
        if report.starts_with('{') {
            assert_eq!(status["status"], "rejected");
        } else {
            assert!(status.is_string());
        }
    }
}

#[test]
fn accepted_training_adopts_only_candidate_owned_models_and_prunes_old_versions() {
    let home = Home::new();
    let service = fake_service(&home, "");
    let mut entries = vec![];
    for name in ["whisper-ft-old", "whisper-ft-test", "whisper-ft-accepted"] {
        let path = home.0.join("models").join(name);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("model.bin"), "weights").unwrap();
        if name != "whisper-ft-accepted" {
            entries.push(json!({"name":name,"path":path,"base":"whisper-large-v3-turbo"}));
        }
    }
    service
        .store
        .lock()
        .unwrap()
        .state("versions", &json!(entries))
        .unwrap();
    service.config.lock().unwrap()["keep_versions"] = json!(1);
    let report = json!({"status":"accepted","model":home.0.join("models/whisper-ft-accepted"),"gate1":{"passed":true},"gate2":{"passed":true}});
    training_environment(&home, &report.to_string());
    start_training_on_power(service.clone(), true).unwrap();
    wait_until(|| !service.learning_busy.load(Ordering::SeqCst));
    assert_eq!(
        service.config.lock().unwrap()["model"],
        "ft:whisper-ft-accepted"
    );
    assert_eq!(
        crate::config::load(&home.0).unwrap()["model"],
        "ft:whisper-ft-accepted"
    );
    assert!(!home.0.join("models/whisper-ft-old").exists());
    assert!(!home.0.join("models/whisper-ft-test").exists());
    assert!(home.0.join("models/whisper-ft-accepted/model.bin").exists());
    assert_eq!(
        service
            .store
            .lock()
            .unwrap()
            .get_state("versions")
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn trainer_cannot_adopt_a_model_outside_candidate_data() {
    let outside = Home::new();
    let home = Home::new();
    let service = fake_service(&home, "");
    let name = "whisper-ft-external";
    std::fs::create_dir_all(outside.0.join(name)).unwrap();
    std::fs::create_dir_all(home.0.join("models").join(name)).unwrap();
    let report = json!({"status":"accepted","model":outside.0.join(name),"gate1":{"passed":true},"gate2":{"passed":true}});
    training_environment(&home, &report.to_string());
    start_training_on_power(service.clone(), true).unwrap();
    wait_until(|| !service.learning_busy.load(Ordering::SeqCst));
    assert_eq!(
        service.config.lock().unwrap()["model"],
        "ft:whisper-ft-test"
    );
    assert!(service
        .store
        .lock()
        .unwrap()
        .get_state("training_status")
        .unwrap()
        .as_str()
        .unwrap()
        .contains("external path"));
    assert!(outside.0.join(name).is_dir());
}

#[test]
fn malformed_accepted_reports_and_damaged_training_environments_preserve_the_current_model() {
    for problem in [
        "missing-model",
        "root-path",
        "missing-directory",
        "external-missing",
        "invalid-ledger",
        "damaged-executable",
    ] {
        let home = Home::new();
        let outside = Home::new();
        let service = fake_service(&home, "");
        let before = service.config.lock().unwrap().clone();
        let destination = home.0.join("models/whisper-ft-candidate");
        if problem != "missing-directory" {
            std::fs::create_dir_all(&destination).unwrap();
            std::fs::write(destination.join("model.bin"), "weights").unwrap();
        }
        let mut report = json!({"status":"accepted","model":destination,"gate1":{"passed":true},"gate2":{"passed":true}});
        match problem {
            "missing-model" => {
                report.as_object_mut().unwrap().remove("model");
            }
            "root-path" => report["model"] = json!("/"),
            "external-missing" => report["model"] = json!(outside.0.join("whisper-ft-candidate")),
            "invalid-ledger" => service
                .store
                .lock()
                .unwrap()
                .state("versions", &json!({"damaged":true}))
                .unwrap(),
            _ => (),
        }
        training_environment(&home, &report.to_string());
        if problem == "damaged-executable" {
            let executable = home.0.join(if cfg!(windows) {
                "environments/training/Scripts/python.exe"
            } else {
                "environments/training/bin/python"
            });
            std::fs::write(executable, "This is not an executable").unwrap();
        }
        start_training_on_power(service.clone(), true).unwrap();
        wait_until(|| !service.learning_busy.load(Ordering::SeqCst));
        assert_eq!(*service.config.lock().unwrap(), before, "adopted {problem}");
        assert_eq!(crate::config::load(&home.0).unwrap(), before);
        assert!(service
            .store
            .lock()
            .unwrap()
            .get_state("training_status")
            .unwrap()
            .is_string());
        assert_eq!(service.training_pid.load(Ordering::SeqCst), 0);
        assert!(home.0.join("models/whisper-ft-test/model.bin").exists());
    }
}

#[test]
fn version_retention_protects_the_active_model_and_refuses_damaged_directory_entries() {
    let home = Home::new();
    let service = fake_service(&home, "");
    let active = json!({"name":"whisper-ft-test","path":home.0.join("models/whisper-ft-test")});
    service
        .store
        .lock()
        .unwrap()
        .state("versions", &json!([active, active]))
        .unwrap();
    prune(&service, 1).unwrap();
    assert_eq!(
        service
            .store
            .lock()
            .unwrap()
            .get_state("versions")
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let damaged = home.0.join("models/old-version");
    std::fs::write(&damaged, "An unrelated file").unwrap();
    for path in [damaged.clone(), std::path::PathBuf::from("/")] {
        let ledger = json!([{"name":"old-version","path":path}, active]);
        service
            .store
            .lock()
            .unwrap()
            .state("versions", &ledger)
            .unwrap();
        assert!(prune(&service, 1).is_err());
        assert_eq!(
            service.store.lock().unwrap().get_state("versions").unwrap(),
            ledger
        );
        assert_eq!(
            std::fs::read_to_string(&damaged).unwrap(),
            "An unrelated file"
        );
        assert!(home.0.join("models/whisper-ft-test/model.bin").exists());
    }
}

#[test]
fn the_background_scheduler_starts_due_training_and_preserves_a_rejected_models_predecessor() {
    let home = Home::new();
    let service = fake_service(&home, "");
    training_environment(
        &home,
        r#"{"status":"rejected","gate1":{"passed":false},"gate2":{"passed":true}}"#,
    );
    let before = service.config.lock().unwrap()["model"].clone();
    service.config.lock().unwrap()["teacher"] = json!("off");
    {
        let mut store = service.store.lock().unwrap();
        for _ in 0..50 {
            let id = store
                .record(&[0.1; 160], "old words", "old words", true)
                .unwrap();
            store.edit(id, "human correction").unwrap();
        }
        store
            .state("last_training", &json!({"count":0,"ts":now()-10.*86400.}))
            .unwrap();
    }
    *service.last_activity.lock().unwrap() = Instant::now() - Duration::from_secs(600);
    let on_mains = mains();
    let task = scheduler(service.clone());
    if on_mains {
        wait_until(|| {
            service
                .store
                .lock()
                .unwrap()
                .get_state("last_training")
                .unwrap()["count"]
                == 50
        });
        assert_eq!(
            service
                .store
                .lock()
                .unwrap()
                .get_state("training_status")
                .unwrap()["status"],
            "rejected"
        );
        assert_eq!(service.config.lock().unwrap()["model"], before);
    } else {
        std::thread::sleep(Duration::from_secs(6));
        assert_eq!(
            service
                .store
                .lock()
                .unwrap()
                .get_state("last_training")
                .unwrap()["count"],
            0
        );
    }
    service.shutdown();
    task.join().unwrap();
}

#[test]
fn rollback_uses_the_previous_adopted_version_and_keeps_the_version_ledger() {
    let home = Home::new();
    let service = fake_service(&home, "");
    let older = home.0.join("models/whisper-ft-older");
    std::fs::create_dir_all(&older).unwrap();
    std::fs::write(older.join("model.bin"), "weights").unwrap();
    let ledger = json!([{"name":"whisper-ft-older","base":"whisper-large-v3-turbo"},{"name":"whisper-ft-test","base":"whisper-large-v3-turbo"}]);
    service
        .store
        .lock()
        .unwrap()
        .state("versions", &ledger)
        .unwrap();
    rollback(&service).unwrap();
    assert_eq!(
        service.config.lock().unwrap()["model"],
        "ft:whisper-ft-older"
    );
    assert_eq!(
        service.store.lock().unwrap().get_state("versions").unwrap(),
        ledger
    );
    // Uninstalled base selection is refused and restores the working version.
    assert!(rollback(&service).is_err());
    assert_eq!(
        service.config.lock().unwrap()["model"],
        "ft:whisper-ft-older"
    );
    service
        .store
        .lock()
        .unwrap()
        .state("versions", &json!({}))
        .unwrap();
    assert!(rollback(&service).is_err());
    service
        .store
        .lock()
        .unwrap()
        .state("versions", &json!([]))
        .unwrap();
    assert!(rollback(&service).is_err());
}

#[test]
fn battery_power_prevents_training_without_changing_the_active_model() {
    let home = Home::new();
    let service = fake_service(&home, "");
    training_environment(&home, "must not run");
    let before = service.config.lock().unwrap().clone();
    assert!(start_training_on_power(service.clone(), false)
        .unwrap_err()
        .contains("battery"));
    assert!(!service.learning_busy.load(Ordering::SeqCst));
    assert_eq!(*service.config.lock().unwrap(), before);
    assert!(!home.0.join("training-report.json").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn linux_power_reading_distinguishes_mains_battery_desktops_and_unavailable_status() {
    let home = Home::new();
    assert!(!linux_mains(&home.0.join("absent")));
    assert!(linux_mains(&home.0));
    let battery = home.0.join("BAT0");
    std::fs::create_dir(&battery).unwrap();
    std::fs::write(battery.join("type"), "Battery\n").unwrap();
    std::fs::write(battery.join("status"), "Discharging\n").unwrap();
    assert!(!linux_mains(&home.0));
    std::fs::write(battery.join("status"), "Charging\n").unwrap();
    assert!(linux_mains(&home.0));
    std::fs::write(battery.join("status"), "Full\n").unwrap();
    assert!(linux_mains(&home.0));
    std::fs::write(battery.join("status"), "Unknown\n").unwrap();
    let adapter = home.0.join("AC");
    std::fs::create_dir(&adapter).unwrap();
    std::fs::write(adapter.join("online"), "1\n").unwrap();
    assert!(linux_mains(&home.0));
    std::fs::write(adapter.join("online"), "0\n").unwrap();
    assert!(!linux_mains(&home.0));
}

fn idle(service: &Arc<Service>) -> (Value, Instant) {
    *service.last_activity.lock().unwrap() = Instant::now() - Duration::from_secs(600);
    let mut cfg = service.config.lock().unwrap().clone();
    cfg["teacher"] = json!("off");
    (cfg, Instant::now() - Duration::from_secs(1))
}

#[test]
fn scheduler_pauses_when_busy_recently_active_disabled_or_on_battery() {
    let home = Home::new();
    let service = home.service();
    let (mut cfg, mut retry) = idle(&service);
    scheduler_tick(&service, &cfg, false, &mut retry);
    assert!(service
        .store
        .lock()
        .unwrap()
        .get_state("teacher_status")
        .is_err());
    service.learning_busy.store(true, Ordering::SeqCst);
    scheduler_tick(&service, &cfg, false, &mut retry);
    assert_eq!(service.snapshot()["state"]["stage"], "Cancelled");
    service.learning_busy.store(false, Ordering::SeqCst);
    cfg["teacher"] = json!("unknown-model");
    for reason in [
        "download",
        "recording",
        "learning",
        "disabled",
        "recent",
        "backoff",
    ] {
        service
            .downloading
            .store(reason == "download", Ordering::SeqCst);
        service
            .recording
            .store(reason == "recording", Ordering::SeqCst);
        service
            .learning_busy
            .store(reason == "learning", Ordering::SeqCst);
        cfg["learn_auto"] = json!(reason != "disabled");
        *service.last_activity.lock().unwrap() =
            Instant::now() - Duration::from_secs(if reason == "recent" { 0 } else { 600 });
        retry = if reason == "backoff" {
            Instant::now() + Duration::from_secs(60)
        } else {
            Instant::now() - Duration::from_secs(1)
        };
        scheduler_tick(&service, &cfg, true, &mut retry);
        assert!(
            service
                .store
                .lock()
                .unwrap()
                .get_state("teacher_status")
                .is_err(),
            "scheduler ignored {reason}"
        );
    }
}

#[test]
fn teacher_failure_is_reported_and_backed_off_without_starting_a_download() {
    let home = Home::new();
    let service = home.service();
    let (mut cfg, mut retry) = idle(&service);
    cfg["teacher"] = json!("unknown-model");
    cfg["teacher_on_cpu"] = json!(true);
    cfg["device"] = json!("cpu");
    scheduler_tick(&service, &cfg, true, &mut retry);
    assert!(service
        .store
        .lock()
        .unwrap()
        .get_state("teacher_status")
        .unwrap()
        .as_str()
        .unwrap()
        .contains("Unknown catalog model"));
    assert!(!service.downloading.load(Ordering::SeqCst));
    assert!(retry > Instant::now() + Duration::from_secs(3500));
    let status = service
        .store
        .lock()
        .unwrap()
        .get_state("teacher_status")
        .unwrap();
    cfg["teacher"] = json!("off");
    scheduler_tick(&service, &cfg, true, &mut retry);
    assert_eq!(
        service
            .store
            .lock()
            .unwrap()
            .get_state("teacher_status")
            .unwrap(),
        status
    );
}

#[test]
fn scheduler_labels_unedited_audio_and_respects_explicit_cpu_consent() {
    let home = Home::new();
    let service = fake_service(&home, "");
    let (mut cfg, mut retry) = idle(&service);
    cfg["teacher"] = json!("distil-whisper-large-v3");
    cfg["device"] = json!("cpu");
    scheduler_tick(&service, &cfg, true, &mut retry);
    assert!(service
        .store
        .lock()
        .unwrap()
        .get_state("teacher_status")
        .unwrap()
        .as_str()
        .unwrap()
        .contains("teacher_on_cpu"));
    cfg["teacher_on_cpu"] = json!(true);
    retry = Instant::now() - Duration::from_secs(1);
    scheduler_tick(&service, &cfg, true, &mut retry);
    assert!(service
        .store
        .lock()
        .unwrap()
        .get_state("teacher_status")
        .unwrap()
        .as_str()
        .unwrap()
        .contains("Install teacher"));
    std::fs::write(home.0.join("models/distil-whisper-large-v3.gguf"), "marker").unwrap();
    std::fs::create_dir_all(home.0.join("models/distil-whisper-large-v3")).unwrap();
    let id = service
        .store
        .lock()
        .unwrap()
        .record(&[0.1; 16000], "student guess", "student guess", true)
        .unwrap();
    retry = Instant::now() - Duration::from_secs(1);
    scheduler_tick(&service, &cfg, true, &mut retry);
    let rows = service.store.lock().unwrap().history("", "").unwrap();
    assert_eq!(rows[0]["id"], id);
    assert_eq!(rows[0]["teacher_text"], "get hub");
    assert_eq!(rows[0]["teacher_model"], "distil-whisper-large-v3");
    assert_eq!(rows[0]["pasted"], "student guess");
    assert!(!service.learning_busy.load(Ordering::SeqCst));
    scheduler_tick(&service, &cfg, true, &mut retry);
    assert_eq!(
        service.store.lock().unwrap().history("", "").unwrap()[0]["teacher_text"],
        "get hub"
    );
}

#[test]
fn shutdown_releases_the_background_scheduler_without_starting_work() {
    let home = Home::new();
    let service = home.service();
    let task = scheduler(service.clone());
    service.shutdown();
    task.join().unwrap();
    assert!(service
        .store
        .lock()
        .unwrap()
        .get_state("last_training")
        .is_err());
}

#[test]
fn retraining_a_catalog_whisper_passes_its_base_as_the_current_model_and_preserves_it_after_rejection(
) {
    for base in ["whisper-large-v3", "whisper-large-v3-turbo"] {
        let home = Home::new();
        let service = home.service();
        service.config.lock().unwrap()["model"] = json!(base);
        training_environment(
            &home,
            r#"{"status":"rejected","gate1":{"passed":false},"gate2":{"passed":true}}"#,
        );
        start_training_on_power(service.clone(), true).unwrap();
        wait_until(|| !service.learning_busy.load(Ordering::SeqCst));
        let args = std::fs::read_to_string(home.0.join("trainer-args.txt")).unwrap();
        let args: Vec<_> = args.lines().collect();
        for key in ["--base", "--current"] {
            assert_eq!(
                args[args.iter().position(|value| *value == key).unwrap() + 1],
                format!("openai/{base}")
            );
        }
        assert_eq!(service.config.lock().unwrap()["model"], base);
        assert_eq!(
            service
                .store
                .lock()
                .unwrap()
                .get_state("training_status")
                .unwrap()["status"],
            "rejected"
        );
    }
}

#[test]
fn retention_drops_missing_versions_without_deleting_the_active_model() {
    let home = Home::new();
    let service = fake_service(&home, "");
    let active = json!({"name":"whisper-ft-test", "path":home.0.join("models/whisper-ft-test")});
    service
        .store
        .lock()
        .unwrap()
        .state(
            "versions",
            &json!([{"name":"missing-old", "path":home.0.join("models/missing-old")}, active]),
        )
        .unwrap();
    prune(&service, 1).unwrap();
    assert_eq!(
        service.store.lock().unwrap().get_state("versions").unwrap(),
        json!([active])
    );
    assert!(home.0.join("models/whisper-ft-test/model.bin").exists());
}

#[test]
fn teacher_work_is_skipped_while_another_job_is_busy_and_failed_inference_never_creates_a_label() {
    let home = Home::new();
    let service = fake_service(&home, "");
    let mut cfg = service.config.lock().unwrap().clone();
    cfg["teacher"] = json!("distil-whisper-large-v3");
    cfg["teacher_on_cpu"] = json!(true);
    std::fs::write(home.0.join("models/distil-whisper-large-v3.gguf"), "marker").unwrap();
    let model = home.0.join("models/distil-whisper-large-v3");
    std::fs::create_dir(&model).unwrap();
    std::fs::write(model.join("scenario"), "error").unwrap();
    service
        .store
        .lock()
        .unwrap()
        .record(&[0.25; 16000], "a student guess", "a student guess", true)
        .unwrap();
    service.learning_busy.store(true, Ordering::SeqCst);
    teacher(&service, &cfg).unwrap();
    assert_eq!(service.pid.load(Ordering::SeqCst), 0);
    assert!(service.store.lock().unwrap().history("", "").unwrap()[0]["teacher_text"].is_null());
    service.learning_busy.store(false, Ordering::SeqCst);
    assert!(teacher(&service, &cfg).unwrap_err().contains("bad clip"));
    assert!(!service.learning_busy.load(Ordering::SeqCst));
    assert!(service.store.lock().unwrap().history("", "").unwrap()[0]["teacher_text"].is_null());
    assert_eq!(
        service.store.lock().unwrap().history("", "").unwrap()[0]["pasted"],
        "a student guess"
    );
}

#[cfg(unix)]
#[test]
fn an_optional_teacher_falling_back_to_cpu_requires_consent_and_never_labels_speech_without_it() {
    use std::os::unix::fs::PermissionsExt;
    let home = Home::new();
    if std::env::var_os("SPEECHIFY_TEST_TEACHER_GPU_CHILD").is_none() {
        let tools = home.0.join("tools");
        std::fs::create_dir(&tools).unwrap();
        let query = tools.join("nvidia-smi");
        std::fs::write(&query, "#!/bin/sh\nprintf '16384\\n'\n").unwrap();
        std::fs::set_permissions(&query, std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = std::env::join_paths(
            std::iter::once(tools).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
        )
        .unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "learning::behavior::an_optional_teacher_falling_back_to_cpu_requires_consent_and_never_labels_speech_without_it"])
            .env("SPEECHIFY_TEST_TEACHER_GPU_CHILD", "1").env("PATH", path).output().unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let service = fake_service(&home, "");
    let mut cfg = service.config.lock().unwrap().clone();
    cfg["teacher"] = json!("distil-whisper-large-v3");
    cfg["device"] = json!("auto");
    cfg["teacher_on_cpu"] = json!(false);
    std::fs::write(
        home.0.join("models/distil-whisper-large-v3.gguf"),
        "installed marker",
    )
    .unwrap();
    std::fs::create_dir(home.0.join("models/distil-whisper-large-v3")).unwrap();
    service
        .store
        .lock()
        .unwrap()
        .record(&[0.1; 16000], "student guess", "student guess", true)
        .unwrap();
    assert!(teacher(&service, &cfg)
        .unwrap_err()
        .contains("CPU requires explicit opt-in"));
    assert!(!service.learning_busy.load(Ordering::SeqCst));
    assert!(service.store.lock().unwrap().history("", "").unwrap()[0]["teacher_text"].is_null());
    assert_eq!(
        service.store.lock().unwrap().history("", "").unwrap()[0]["pasted"],
        "student guess"
    );
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "Requires the delayed-IO fixture from test-coverage.sh"]
fn cancelling_a_teacher_with_a_completed_but_delayed_reply_never_adopts_the_label() {
    use crate::behavior_tests::{io_child, IoGate};
    if !io_child("delay-transcript-read") {
        return;
    }
    let home = Home::new();
    let service = fake_service(&home, "");
    let mut cfg = service.config.lock().unwrap().clone();
    cfg["teacher"] = json!("distil-whisper-large-v3");
    cfg["teacher_on_cpu"] = json!(true);
    std::fs::write(
        home.0.join("models/distil-whisper-large-v3.gguf"),
        "installed marker",
    )
    .unwrap();
    std::fs::create_dir(home.0.join("models/distil-whisper-large-v3")).unwrap();
    service
        .store
        .lock()
        .unwrap()
        .record(&[0.25; 16000], "student guess", "student guess", true)
        .unwrap();
    let gate = IoGate::new(&home);
    let background = service.clone();
    let job = std::thread::spawn(move || teacher(&background, &cfg));
    wait_until(|| gate.paused.exists());
    service.cancel_work();
    gate.release();
    assert_eq!(job.join().unwrap().unwrap_err(), "Teacher preempted");
    let history = service.store.lock().unwrap().history("", "").unwrap();
    assert!(history[0]["teacher_text"].is_null());
    assert_eq!(history[0]["pasted"], "student guess");
    assert!(!service.learning_busy.load(Ordering::SeqCst));
}
