use super::*;
use crate::behavior_tests::{fake_service, training_environment, wait_until, Home};

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
