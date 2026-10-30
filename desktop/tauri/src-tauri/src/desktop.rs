use crate::{
    audio::capture::Capture,
    config::Result,
    keys::{Action, Hold},
    platform,
    service::Service,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    path::Path,
    sync::{atomic::Ordering, mpsc, Arc},
    time::{Duration, Instant},
};
use tauri::{Emitter, Manager};
#[cfg(test)]
#[path = "../tests/unit/desktop.rs"]
mod behavior;
#[tauri::command]
async fn request(
    op: String,
    args: Value,
    service: tauri::State<'_, Arc<Service>>,
) -> Result<Value> {
    let s = service.inner().clone();
    tauri::async_runtime::spawn_blocking(move || dispatch(&s, &op, &args))
        .await
        .map_err(crate::config::err)?
}
fn dispatch(s: &Arc<Service>, op: &str, args: &Value) -> Result<Value> {
    match op {
        "snapshot" => Ok(s.snapshot()),
        "manual_start" => {
            if s.learning_busy.load(Ordering::SeqCst) {
                s.cancel_work();
            }
            let mut started = s.manual_started.lock().unwrap();
            if s.recording.swap(true, Ordering::SeqCst) {
                return Err("A recording is already active".into());
            }
            let result = s
                .capture_tx
                .lock()
                .unwrap()
                .as_ref()
                .ok_or("Recorder unavailable")
                .and_then(|tx| {
                    tx.try_send((Action::Start, 0, s.generation.load(Ordering::SeqCst)))
                        .map_err(|_| "Recorder busy")
                });
            if let Err(e) = result {
                s.recording.store(false, Ordering::SeqCst);
                return Err(e.into());
            }
            *started = Some(Instant::now());
            *s.last_activity.lock().unwrap() = Instant::now();
            Ok(json!(true))
        }
        "manual_stop" => {
            let start = s
                .manual_started
                .lock()
                .unwrap()
                .take()
                .ok_or("No app recording is active")?;
            let minimum = s.config.lock().unwrap()["min_hold_ms"]
                .as_u64()
                .unwrap_or(250);
            let action = if start.elapsed().as_millis() < minimum as u128 {
                Action::Cancel
            } else {
                Action::Submit
            };
            s.recording.store(false, Ordering::SeqCst);
            s.capture_tx
                .lock()
                .unwrap()
                .as_ref()
                .ok_or("Recorder unavailable")?
                .try_send((action, 0, s.generation.load(Ordering::SeqCst)))
                .map_err(|_| "Recorder busy")?;
            Ok(json!(true))
        }
        "devices" => Ok(json!(crate::audio::capture::devices()?)),
        "settings" => {
            s.update(args.clone())?;
            Ok(json!(true))
        }
        "history" => Ok(json!(s.store.lock().unwrap().history(
            args["search"].as_str().unwrap_or(""),
            args["filter"].as_str().unwrap_or("")
        )?)),
        "correct" => {
            let sample = id(args)?;
            let text = args["text"].as_str().ok_or("Missing correction")?;
            // Learning is saved even when OS replacement is refused.
            s.store.lock().unwrap().edit(sample, text)?;
            if args["replace"] == true {
                let receipt = {
                    let mut slot = s.replacement.lock().unwrap();
                    if slot.as_ref().is_some_and(|(saved, _)| *saved == sample) {
                        slot.take()
                    } else {
                        None
                    }
                };
                let Some((_, receipt)) = receipt else {
                    return Err("Correction saved. Replacement unavailable: no proven original text range/caret for this sample. Copy it manually.".into());
                };
                crate::replacement::replace(&receipt, text)?;
            }
            Ok(json!(true))
        }
        "revert" => {
            s.store.lock().unwrap().revert(id(args)?)?;
            Ok(json!(true))
        }
        "exclude" => {
            s.store
                .lock()
                .unwrap()
                .exclude(id(args)?, args["excluded"] == true)?;
            Ok(json!(true))
        }
        "delete" => {
            s.store.lock().unwrap().delete(id(args)?)?;
            Ok(json!(true))
        }
        "audio" => {
            use base64::Engine;
            let p = s.store.lock().unwrap().audio_path(id(args)?)?;
            let bytes = std::fs::read(p).map_err(crate::config::err)?;
            Ok(json!(
                base64::engine::general_purpose::STANDARD.encode(bytes)
            ))
        }
        "copy" => {
            platform::write_clipboard(args["text"].as_str().ok_or("Missing text")?)?;
            Ok(json!(true))
        }
        "learning" => s.store.lock().unwrap().learning(),
        "forget" => {
            s.store.lock().unwrap().forget(
                args["kind"].as_str().unwrap_or(""),
                args["key"].as_str().unwrap_or(""),
            )?;
            Ok(json!(true))
        }
        "export" => {
            let threshold = s.config.lock().unwrap()["teacher_min_confidence"]
                .as_f64()
                .unwrap_or(0.7);
            Ok(json!(s.store.lock().unwrap().export(threshold)?))
        }
        "import" => {
            let source = Path::new(args["path"].as_str().ok_or("Source directory required")?);
            Ok(json!(s.store.lock().unwrap().import(source)?))
        }
        "export_config" => Ok(json!(
            toml::to_string_pretty(&*s.config.lock().unwrap()).map_err(crate::config::err)?
        )),
        "import_config" => {
            let raw: toml::Value = toml::from_str(args["text"].as_str().ok_or("TOML required")?)
                .map_err(crate::config::err)?;
            let mut cfg = crate::config::defaults();
            for (k, v) in serde_json::to_value(raw)
                .map_err(crate::config::err)?
                .as_object()
                .ok_or("TOML table required")?
            {
                cfg[k] = v.clone();
            }
            s.update(cfg)?;
            Ok(json!(true))
        }
        "install" => {
            if s.learning_busy.load(Ordering::SeqCst) {
                return Err("Pause learning before installing models".into());
            }
            let id = args["id"].as_str().ok_or("Model ID required")?.to_owned();
            if id != "training" {
                crate::models::spec(&id)?;
            }
            if s.downloading.swap(true, Ordering::SeqCst) {
                return Err(
                    "One model download at a time; cancel or wait for the current download".into(),
                );
            }
            s.cancel.store(false, Ordering::SeqCst);
            let service = s.clone();
            std::thread::spawn(move || {
                let optional =
                    id == "training" || crate::models::spec(&id).is_ok_and(|m| m["gguf"].is_null());
                let result = if optional {
                    crate::installer::install(&service, &id)
                } else {
                    crate::models::install(&service.root, &id, service.cancel.clone(), |p| {
                        service.state.lock().unwrap()["download"] = p
                    })
                };
                service.downloading.store(false, Ordering::SeqCst);
                match result {
                    Ok(()) => {
                        service.state.lock().unwrap()["download"] =
                            json!({"id":id,"stage":"Installed"});
                    }
                    Err(e) => {
                        service.state.lock().unwrap()["download"] =
                            json!({"id":id,"stage":"Failed","error":e})
                    }
                }
            });
            Ok(json!(true))
        }
        "cancel" => {
            s.cancel_work();
            Ok(json!(true))
        }
        "load" => {
            s.preload()?;
            Ok(json!(true))
        }
        "reuse_model" => {
            let id = args["id"].as_str().ok_or("Model ID required")?;
            let source = Path::new(args["path"].as_str().ok_or("Source path required")?);
            let spec = crate::models::spec(id)?;
            let hash = spec["gguf"]["sha256"]
                .as_str()
                .ok_or("Native GGUF model required")?;
            crate::models::verify(source, hash, &std::sync::atomic::AtomicBool::new(false))?;
            let dest = crate::models::path(&s.root, id)?;
            let tmp = dest.with_extension("partial");
            std::fs::copy(source, &tmp).map_err(crate::config::err)?;
            crate::models::verify(&tmp, hash, &std::sync::atomic::AtomicBool::new(false))?;
            std::fs::rename(tmp, dest).map_err(crate::config::err)?;
            Ok(json!(true))
        }
        "train" => {
            crate::learning::start_training(s.clone())?;
            Ok(json!(true))
        }
        "rollback" => {
            crate::learning::rollback(s)?;
            Ok(json!(true))
        }
        "autostart" => {
            autostart(args["enabled"] == true)?;
            Ok(json!(true))
        }
        _ => Err("Unknown candidate command".into()),
    }
}
fn id(v: &Value) -> Result<i64> {
    v["id"].as_i64().ok_or("Sample ID required".into())
}
fn show(app: &tauri::AppHandle, page: &str) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
        let _ = w.emit("navigate", page);
    }
}
fn runtime(app: tauri::AppHandle, s: Arc<Service>, indicator: tauri::WebviewWindow) {
    start_runtime(s, move |page| show(&app, page), move |visible| {
        if visible { let _ = indicator.show(); } else { let _ = indicator.hide(); }
    }, Capture::open);
}
fn start_runtime(
    s: Arc<Service>,
    navigate: impl Fn(&str) + Send + 'static,
    indicator: impl Fn(bool) + Send + 'static,
    open_capture: impl Fn(&str) -> Result<Capture> + Send + 'static,
) -> [std::thread::JoinHandle<()>; 3] {
    let (tx, rx) = mpsc::sync_channel::<(Vec<f32>, u64, u64)>(4);
    let infer = s.clone();
    let inference = std::thread::spawn(move || {
        for (audio, target, generation) in rx {
            if infer.stopping.load(Ordering::SeqCst) {
                break;
            }
            if infer.generation.load(Ordering::SeqCst) != generation {
                continue;
            }
            let path = infer.root.join(format!(
                "pending-{}.wav",
                (crate::store::now() * 1e6) as u64
            ));
            let result: Result<()> = (|| {
                crate::audio::save(&path, &audio)?;
                let result = infer.infer_path(&path, true)?;
                if infer.generation.load(Ordering::SeqCst) != generation {
                    return Err("Cancelled".into());
                }
                let text = result["text"].as_str().unwrap_or("");
                if !text.is_empty() {
                    if target == 0 {
                        infer.state.lock().unwrap()["error"] = json!("Transcript ready. Use Copy text to insert it; no safe external target was established.");
                        return Ok(());
                    }
                    let receipt = platform::paste(target, text)?;
                    *infer.replacement.lock().unwrap() = result["sample_id"].as_i64().zip(receipt);
                    infer.set_stage("Pasted");
                    infer.log("paste_ack", json!({"clock_ms":crate::store::now()*1000.}));
                }
                Ok(())
            })();
            let _ = std::fs::remove_file(path);
            if let Err(e) = result {
                if infer.generation.load(Ordering::SeqCst) == generation {
                    infer.error(e);
                }
            }
        }
    });
    let (capture_tx, capture_rx) = mpsc::sync_channel::<(Action, u64, u64)>(32);
    *s.capture_tx.lock().unwrap() = Some(capture_tx.clone());
    let recorder = s.clone();
    let recording = std::thread::spawn(move || {
        let mut capture: Option<Capture> = None;
        let mut active_target = 0;
        let mut active_capture = false;
        let mut active_generation = 0;
        let mut last_input = String::new();
        let mut hide_at = None;
        while !recorder.stopping.load(Ordering::SeqCst) {
            let command = capture_rx.recv_timeout(Duration::from_millis(15));
            let cfg = recorder.config.lock().unwrap().clone();
            let keep = cfg["keep_mic_open"] == true;
            let input = cfg["input_device"].as_str().unwrap_or("");
            if last_input != input {
                capture = None;
                last_input = input.into();
            }
            if let Ok((action, target, generation)) = command {
                match action {
                    Action::Start => {
                        active_capture = true;
                        active_target = target;
                        active_generation = generation;
                        if capture.is_none() {
                            match open_capture(input) {
                                Ok(c) => {
                                    recorder.state.lock().unwrap()["actual_input"] =
                                        json!(&c.actual_device);
                                    capture = Some(c);
                                }
                                Err(e) => {
                                    recorder.error(e);
                                    recorder.recording.store(false, Ordering::SeqCst);
                                }
                            }
                        }
                        if capture.is_some() && recorder.recording.load(Ordering::SeqCst) {
                            recorder.set_stage("Recording");
                            recorder.log(
                                "microphone_ready",
                                json!({"clock_ms":crate::store::now()*1000.}),
                            );
                            indicator(true);
                        }
                    }
                    Action::Cancel => {
                        active_capture = false;
                        if keep {
                            if let Some(c) = &capture {
                                c.audio.clear();
                            }
                        } else {
                            capture = None;
                        }
                        recorder.set_stage("Ready");
                        indicator(false);
                    }
                    Action::Submit => {
                        if !active_capture {
                            continue;
                        }
                        active_capture = false;
                        if let Some(c) = &capture {
                            let taken = c.audio.take();
                            let stream_failed = taken.is_err();
                            let queued = taken.and_then(|audio| {
                                if audio.len() < 160 {
                                    return Err("Microphone was not ready before release; try holding longer".into());
                                }
                                tx.try_send((audio, active_target, active_generation))
                                    .map_err(|_| "Dictation queue is full (4 clips); wait for transcription".into())
                            });
                            match queued {
                                Ok(()) => recorder.set_stage("Queued"),
                                Err(e) => recorder.error(e),
                            }
                            if stream_failed {
                                capture = None;
                            }
                        }
                        if !keep {
                            capture = None;
                        }
                        hide_at = Some(Instant::now() + Duration::from_secs(3));
                    }
                }
            }
            if keep && capture.is_none() && !recorder.recording.load(Ordering::SeqCst) {
                match open_capture(input) {
                    Ok(c) => {
                        recorder.state.lock().unwrap()["actual_input"] = json!(&c.actual_device);
                        capture = Some(c);
                    }
                    Err(e) => {
                        recorder.error(e);
                        std::thread::sleep(Duration::from_secs(1));
                    }
                }
            }
            if !keep && !active_capture {
                capture = None;
            }
            if let Some(c) = &capture {
                if !active_capture {
                    c.audio.preroll();
                }
                recorder.state.lock().unwrap()["level"] = json!(c.audio.level());
                let err = c.audio.take_error();
                if let Some(e) = err {
                    recorder.error(e);
                    capture = None;
                    recorder.recording.store(false, Ordering::SeqCst);
                }
            } else {
                recorder.state.lock().unwrap()["level"] = json!(0.);
            }
            if hide_at.is_some_and(|t| Instant::now() > t)
                && ["Ready", "Pasted", "Error", "Cancelled"].contains(
                    &recorder.state.lock().unwrap()["stage"]
                        .as_str()
                        .unwrap_or(""),
                )
                && !recorder.recording.load(Ordering::SeqCst)
            {
                indicator(false);
                hide_at = None;
            }
        }
    });
    let keyboard = std::thread::spawn(move || {
        let poll = match platform::Poller::new() {
            Ok(p) => p,
            Err(e) => {
                s.error(e);
                return;
            }
        };
        let mut cfg = s.config.lock().unwrap().clone();
        let mut hold = Hold::new(
            cfg["ptt_key"].as_str().unwrap(),
            cfg["min_hold_ms"].as_u64().unwrap(),
        );
        let mut nav: Vec<_> = ["correct_hotkey", "history_hotkey", "models_hotkey"]
            .into_iter()
            .map(|k| Hold::new(cfg[k].as_str().unwrap(), 0))
            .collect();
        let mut previous = BTreeSet::new();
        let clock = Instant::now();
        while !s.stopping.load(Ordering::SeqCst) {
            let current = s.config.lock().unwrap().clone();
            if current != cfg {
                cfg = current;
                hold = Hold::new(
                    cfg["ptt_key"].as_str().unwrap(),
                    cfg["min_hold_ms"].as_u64().unwrap(),
                );
                nav = ["correct_hotkey", "history_hotkey", "models_hotkey"]
                    .into_iter()
                    .map(|k| Hold::new(cfg[k].as_str().unwrap(), 0))
                    .collect();
                previous.clear();
            }
            let down = match poll.down() {
                Ok(d) => d,
                Err(e) => {
                    s.recording.store(false, Ordering::SeqCst);
                    let _ = capture_tx.try_send((
                        Action::Cancel,
                        0,
                        s.generation.load(Ordering::SeqCst),
                    ));
                    s.error(e);
                    break;
                }
            };
            let events: Vec<_> = previous
                .difference(&down)
                .map(|k| (k.clone(), false))
                .chain(down.difference(&previous).map(|k| (k.clone(), true)))
                .collect();
            for (key, pressed) in events {
                for (i, n) in nav.iter_mut().enumerate() {
                    if n.event(&key, pressed, clock.elapsed().as_millis() as u64)
                        == Some(Action::Start)
                    {
                        navigate(["History", "History", "Models"][i]);
                    }
                }
                if s.manual_started.lock().unwrap().is_some() {
                    continue;
                }
                if let Some(action) = hold.event(&key, pressed, clock.elapsed().as_millis() as u64)
                {
                    let mut target = 0;
                    if action == Action::Start {
                        *s.last_activity.lock().unwrap() = Instant::now();
                        if s.learning_busy.load(Ordering::SeqCst) {
                            s.cancel_work();
                        }
                        target = poll.focus().unwrap_or(0);
                        s.recording.store(true, Ordering::SeqCst);
                    } else {
                        s.recording.store(false, Ordering::SeqCst);
                    }
                    if capture_tx
                        .try_send((action, target, s.generation.load(Ordering::SeqCst)))
                        .is_err()
                    {
                        s.recording.store(false, Ordering::SeqCst);
                        s.error("Microphone command queue full; restart candidate");
                    }
                }
            }
            previous = down;
            std::thread::sleep(Duration::from_millis(8));
        }
    });
    [inference, recording, keyboard]
}

pub fn run() -> Result<()> {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            show(app, "Dictation")
        }))
        .invoke_handler(tauri::generate_handler![request])
        .setup(|app| {
            let s = Service::new(
                crate::config::root().map_err(std::io::Error::other)?,
                crate::worker::native_dir(Some(app.path().resource_dir()?)),
            )
            .map_err(std::io::Error::other)?;
            app.manage(s.clone());
            let icon = tauri::image::Image::new_owned(
                {
                    let mut bytes = vec![0u8; 32 * 32 * 4];
                    for y in 0..32 {
                        for x in 0..32 {
                            let i = (y * 32 + x) * 4;
                            let h = if x % 5 < 3 { 6 + (x * 7) % 19 } else { 0 };
                            if y >= 16 - h / 2 && y <= 16 + h / 2 && h > 0 {
                                bytes[i..i + 4].copy_from_slice(&[141, 183, 255, 255]);
                            }
                        }
                    }
                    bytes
                },
                32,
                32,
            );
            let menu = tauri::menu::Menu::new(app)?;
            for (id, text) in [
                ("settings", "Settings"),
                ("history", "History"),
                ("models", "Models"),
                ("logs", "Log location"),
                ("quit", "Quit candidate"),
            ] {
                menu.append(&tauri::menu::MenuItem::with_id(app, id, text, true, None::<&str>)?)?;
            }
            tauri::tray::TrayIconBuilder::new()
                .icon(icon)
                .tooltip("Speechify Tauri Candidate")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "quit" => {
                        app.state::<Arc<Service>>().shutdown();
                        app.exit(0);
                    }
                    "history" => show(app, "History"),
                    "models" => show(app, "Models"),
                    "logs" => show(app, "Settings"),
                    _ => show(app, "Settings"),
                })
                .build(app)?;
            let indicator = tauri::WebviewWindowBuilder::new(
                app,
                "indicator",
                tauri::WebviewUrl::App("index.html?overlay".into()),
            )
            .title("Speechify recording status")
            .inner_size(280., 68.)
            .decorations(false)
            .resizable(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .focused(false)
            .focusable(false)
            .visible(false)
            .build()?;
            runtime(app.handle().clone(), s.clone(), indicator);
            let _ = crate::learning::scheduler(s.clone());
            std::thread::spawn(move || {
                if let Err(e) = s.preload() {
                    s.error(e);
                }
            });
            Ok(())
        })
        .on_window_event(|w, e| {
            if w.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = e {
                    api.prevent_close();
                    let _ = w.hide();
                }
            }
        });
    let app = builder
        .build(tauri::generate_context!())
        .map_err(crate::config::err)?;
    app.run(|app, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            app.state::<Arc<Service>>().shutdown();
        }
    });
    Ok(())
}
fn autostart(enabled: bool) -> Result<()> {
    let exe = std::env::current_exe().map_err(crate::config::err)?;
    #[cfg(target_os = "linux")]
    {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".config")))
            .ok_or("HOME or XDG_CONFIG_HOME missing")?;
        let path = base.join("autostart/speechify-tauri-candidate.desktop");
        if enabled {
            std::fs::create_dir_all(path.parent().unwrap()).map_err(crate::config::err)?;
            let exe = exe
                .to_string_lossy()
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('`', "\\`")
                .replace('$', "\\$");
            std::fs::write(path,format!("[Desktop Entry]\nType=Application\nName=Speechify Tauri Candidate\nExec=\"{exe}\"\nX-GNOME-Autostart-enabled=true\n")).map_err(crate::config::err)?;
        } else if path.exists() {
            std::fs::remove_file(path).map_err(crate::config::err)?;
        }
    }
    #[cfg(target_os = "macos")]
    {
        let path = std::path::PathBuf::from(std::env::var_os("HOME").ok_or("HOME missing")?)
            .join("Library/LaunchAgents/org.sumedhbhat.speechify.tauri.plist");
        if enabled {
            std::fs::create_dir_all(path.parent().unwrap()).map_err(crate::config::err)?;
            let exe = exe
                .to_string_lossy()
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            std::fs::write(path,format!("<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>Label</key><string>org.sumedhbhat.speechify.tauri</string><key>ProgramArguments</key><array><string>{exe}</string></array><key>RunAtLoad</key><true/></dict></plist>")).map_err(crate::config::err)?;
        } else if path.exists() {
            std::fs::remove_file(path).map_err(crate::config::err)?;
        }
    }
    #[cfg(windows)]
    {
        let key = "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run";
        let status = if enabled {
            std::process::Command::new("reg")
                .args([
                    "add",
                    key,
                    "/v",
                    "SpeechifyTauriCandidate",
                    "/t",
                    "REG_SZ",
                    "/d",
                    &format!("\"{}\"", exe.display()),
                    "/f",
                ])
                .status()
        } else {
            std::process::Command::new("reg")
                .args(["delete", key, "/v", "SpeechifyTauriCandidate", "/f"])
                .status()
        }
        .map_err(crate::config::err)?;
        if !status.success() {
            return Err("Could not update candidate login entry".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod command_tests {
    use super::*;
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
        assert_eq!(rx.recv().unwrap().0, Action::Start);
        dispatch(&s, "manual_stop", &json!({})).unwrap();
        assert_eq!(rx.recv().unwrap().0, Action::Cancel);
        assert!(!s.recording.load(Ordering::SeqCst));
        dispatch(&s, "manual_start", &json!({})).unwrap();
        rx.recv().unwrap();
        dispatch(&s, "cancel", &json!({})).unwrap();
        assert_eq!(rx.recv().unwrap().0, Action::Cancel);
        assert!(s.manual_started.lock().unwrap().is_none());
        let root = s.root.clone();
        drop(s);
        std::fs::remove_dir_all(root).unwrap();
    }
}
