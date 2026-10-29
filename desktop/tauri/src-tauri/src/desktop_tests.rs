use super::*;
use crate::behavior_tests::Home;

#[test]
fn history_commands_save_correct_revert_exclude_export_and_delete() {
    let home = Home::new(); let service = home.service();
    let sample = service.store.lock().unwrap().record(&[0.1;160],"raw","get hub",true).unwrap();
    assert_eq!(dispatch(&service,"snapshot",&json!({})).unwrap()["state"]["stage"],"Idle");
    assert_eq!(dispatch(&service,"history",&json!({"search":"get hub","filter":"unedited"})).unwrap()[0]["id"],sample);
    dispatch(&service,"correct",&json!({"id":sample,"text":"GitHub","replace":false})).unwrap();
    let learning=dispatch(&service,"learning",&json!({})).unwrap();
    assert_eq!(learning["rules"][0]["right"],"GitHub");
    dispatch(&service,"forget",&json!({"kind":"rule","key":"get hub"})).unwrap();
    dispatch(&service,"forget",&json!({"kind":"vocab","key":"GitHub"})).unwrap();
    assert_eq!(dispatch(&service,"learning",&json!({})).unwrap()["rules"],json!([]));
    let output=dispatch(&service,"export",&json!({})).unwrap();
    let metadata=std::fs::read_to_string(Path::new(output.as_str().unwrap()).join("metadata.csv")).unwrap();
    assert!(metadata.contains("GitHub")); assert!(!metadata.contains("get hub"));
    dispatch(&service,"exclude",&json!({"id":sample,"excluded":true})).unwrap();
    assert_eq!(dispatch(&service,"history",&json!({"filter":"excluded"})).unwrap().as_array().unwrap().len(),1);
    dispatch(&service,"revert",&json!({"id":sample})).unwrap();
    assert!(dispatch(&service,"history",&json!({"filter":"edited"})).unwrap().as_array().unwrap().is_empty());
    use base64::Engine;
    let encoded=dispatch(&service,"audio",&json!({"id":sample})).unwrap();
    let wav=base64::engine::general_purpose::STANDARD.decode(encoded.as_str().unwrap()).unwrap();
    assert_eq!(&wav[..4],b"RIFF");
    dispatch(&service,"delete",&json!({"id":sample})).unwrap();
    assert!(dispatch(&service,"history",&json!({})).unwrap().as_array().unwrap().is_empty());
    assert!(dispatch(&service,"audio",&json!({"id":sample})).is_err());
}

#[test]
fn partial_config_import_validates_before_changing_running_or_persisted_settings() {
    let home=Home::new(); let service=home.service();
    let before=dispatch(&service,"export_config",&json!({})).unwrap();
    assert!(before.as_str().unwrap().contains("keep_mic_open = false"));
    dispatch(&service,"import_config",&json!({"text":"theme = 'dark'\nkeep_mic_open = true\n"})).unwrap();
    let cfg=dispatch(&service,"snapshot",&json!({})).unwrap()["config"].clone();
    assert_eq!(cfg["theme"],"dark"); assert_eq!(cfg["keep_mic_open"],true); assert_eq!(cfg["beam_size"],5);
    for bad in ["theme = 'invalid'", "beam_size = 'many'", "broken = ["] {
        assert!(dispatch(&service,"import_config",&json!({"text":bad})).is_err());
        assert_eq!(crate::config::load(&home.0).unwrap(),cfg);
    }
    let mut changed=cfg; changed["theme"]=json!("light");
    dispatch(&service,"settings",&changed).unwrap();
    assert_eq!(crate::config::load(&home.0).unwrap()["theme"],"light");
}

#[test]
fn import_copies_candidate_history_without_modifying_the_source() {
    let source=Home::new(); let original=source.service();
    let id=original.store.lock().unwrap().record(&[0.1;160],"raw","words",true).unwrap();
    let home=Home::new(); let service=home.service();
    assert_eq!(dispatch(&service,"import",&json!({"path":source.0})).unwrap(),1);
    assert_eq!(dispatch(&service,"history",&json!({})).unwrap()[0]["id"],id);
    dispatch(&service,"delete",&json!({"id":id})).unwrap();
    assert_eq!(original.store.lock().unwrap().history("","").unwrap().len(),1);
    assert!(original.store.lock().unwrap().audio_path(id).unwrap().exists());
}

#[test]
fn malformed_commands_are_refused_without_changing_history_or_starting_work() {
    let home=Home::new(); let service=home.service();
    for (command,args) in [
        ("unknown",json!({})),("correct",json!({})),("correct",json!({"id":1})),
        ("audio",json!({"id":"not a number"})),("copy",json!({})),("import",json!({})),
        ("import_config",json!({})),("install",json!({})),("install",json!({"id":"unknown"})),
        ("reuse_model",json!({})),("reuse_model",json!({"id":"moonshine-base"})),
        ("reuse_model",json!({"id":"kyutai-stt-1b","path":"missing"})),
        ("reuse_model",json!({"id":"moonshine-base","path":"missing"})),
        ("load",json!({})),("train",json!({})),("rollback",json!({})),
    ] { assert!(dispatch(&service,command,&args).is_err(),"accepted {command}: {args}"); }
    assert!(!service.downloading.load(Ordering::SeqCst));
    assert!(service.store.lock().unwrap().history("","").unwrap().is_empty());
    service.learning_busy.store(true,Ordering::SeqCst);
    assert!(dispatch(&service,"install",&json!({"id":"moonshine-base"})).unwrap_err().contains("Pause learning"));
    service.learning_busy.store(false,Ordering::SeqCst); service.downloading.store(true,Ordering::SeqCst);
    assert!(dispatch(&service,"install",&json!({"id":"moonshine-base"})).unwrap_err().contains("One model"));
    dispatch(&service,"cancel",&json!({})).unwrap();
    assert_eq!(service.snapshot()["state"]["stage"],"Cancelled");
}

#[test]
fn manual_recording_refuses_unavailable_or_busy_recorder_and_recovers() {
    let home=Home::new(); let service=home.service();
    assert!(dispatch(&service,"manual_stop",&json!({})).unwrap_err().contains("No app recording"));
    assert!(dispatch(&service,"manual_start",&json!({})).unwrap_err().contains("unavailable"));
    assert!(!service.recording.load(Ordering::SeqCst));
    let (tx,rx)=mpsc::sync_channel(1); *service.capture_tx.lock().unwrap()=Some(tx);
    service.learning_busy.store(true,Ordering::SeqCst);
    // Preempting learning sends a cancellation; a full recorder queue refuses
    // the following start instead of claiming the microphone is recording.
    assert!(dispatch(&service,"manual_start",&json!({})).unwrap_err().contains("busy"));
    assert_eq!(rx.recv().unwrap().0,Action::Cancel);
    assert!(!service.recording.load(Ordering::SeqCst));
    service.learning_busy.store(false,Ordering::SeqCst);
    dispatch(&service,"manual_start",&json!({})).unwrap();
    assert_eq!(rx.recv().unwrap().0,Action::Start);
    assert!(service.snapshot()["manual_recording"].as_bool().unwrap());
    assert!(dispatch(&service,"manual_start",&json!({})).unwrap_err().contains("already active"));
    dispatch(&service,"manual_stop",&json!({})).unwrap();
    assert_eq!(rx.recv().unwrap().0,Action::Cancel);
    service.config.lock().unwrap()["min_hold_ms"]=json!(0);
    dispatch(&service,"manual_start",&json!({})).unwrap(); assert_eq!(rx.recv().unwrap().0,Action::Start);
    dispatch(&service,"manual_stop",&json!({})).unwrap(); assert_eq!(rx.recv().unwrap().0,Action::Submit);
    dispatch(&service,"manual_start",&json!({})).unwrap();
    assert!(dispatch(&service,"manual_stop",&json!({})).unwrap_err().contains("busy"));
    assert!(!service.recording.load(Ordering::SeqCst));
}
