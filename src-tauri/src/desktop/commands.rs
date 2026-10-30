//! Settings-window IPC. Validate requests here before calling application services.
use super::runtime::CaptureCommand;
use crate::{config::Result, keys::Action, platform, service::Service};
use serde_json::{json, Value};
use std::{
    path::Path,
    sync::{atomic::Ordering, Arc},
    time::Instant,
};
#[tauri::command]
pub(super) async fn request(
    op: String,
    args: Value,
    service: tauri::State<'_, Arc<Service>>,
) -> Result<Value> {
    let service = service.inner().clone();
    tauri::async_runtime::spawn_blocking(move || dispatch(&service, &op, &args))
        .await
        .map_err(crate::config::err)?
}
pub(super) fn dispatch(service: &Arc<Service>, op: &str, args: &Value) -> Result<Value> {
    match op {
        "snapshot" => Ok(service.snapshot()),
        "manual_start" => {
            if service.learning_busy.load(Ordering::SeqCst) {
                service.cancel_work();
            }
            let mut started = service.manual_started.lock().unwrap();
            if service.recording.swap(true, Ordering::SeqCst) {
                return Err("A recording is already active".into());
            }
            let result = service
                .capture_tx
                .lock()
                .unwrap()
                .as_ref()
                .ok_or("Recorder unavailable")
                .and_then(|tx| {
                    tx.try_send(CaptureCommand {
                        action: Action::Start,
                        target: 0,
                        generation: service.generation.load(Ordering::SeqCst),
                    })
                    .map_err(|_| "Recorder busy")
                });
            if let Err(e) = result {
                service.recording.store(false, Ordering::SeqCst);
                return Err(e.into());
            }
            *started = Some(Instant::now());
            *service.last_activity.lock().unwrap() = Instant::now();
            Ok(json!(true))
        }
        "manual_stop" => {
            let start = service
                .manual_started
                .lock()
                .unwrap()
                .take()
                .ok_or("No app recording is active")?;
            let minimum = service.config.lock().unwrap()["min_hold_ms"]
                .as_u64()
                .unwrap_or(250);
            let action = if start.elapsed().as_millis() < minimum as u128 {
                Action::Cancel
            } else {
                Action::Submit
            };
            service.recording.store(false, Ordering::SeqCst);
            service
                .capture_tx
                .lock()
                .unwrap()
                .as_ref()
                .ok_or("Recorder unavailable")?
                .try_send(CaptureCommand {
                    action: action,
                    target: 0,
                    generation: service.generation.load(Ordering::SeqCst),
                })
                .map_err(|_| "Recorder busy")?;
            Ok(json!(true))
        }
        "devices" => Ok(json!(crate::audio::capture::devices()?)),
        "settings" => {
            service.update(args.clone())?;
            Ok(json!(true))
        }
        "history" => Ok(json!(service.store.lock().unwrap().history(
            args["search"].as_str().unwrap_or(""),
            args["filter"].as_str().unwrap_or("")
        )?)),
        "correct" => {
            let sample = id(args)?;
            let text = args["text"].as_str().ok_or("Missing correction")?;
            // Learning is saved even when OS replacement is refused.
            service.store.lock().unwrap().edit(sample, text)?;
            if args["replace"] == true {
                let receipt = {
                    let mut slot = service.replacement.lock().unwrap();
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
            service.store.lock().unwrap().revert(id(args)?)?;
            Ok(json!(true))
        }
        "exclude" => {
            service
                .store
                .lock()
                .unwrap()
                .exclude(id(args)?, args["excluded"] == true)?;
            Ok(json!(true))
        }
        "delete" => {
            service.store.lock().unwrap().delete(id(args)?)?;
            Ok(json!(true))
        }
        "audio" => {
            use base64::Engine;
            let p = service.store.lock().unwrap().audio_path(id(args)?)?;
            let bytes = std::fs::read(p).map_err(crate::config::err)?;
            Ok(json!(
                base64::engine::general_purpose::STANDARD.encode(bytes)
            ))
        }
        "copy" => {
            platform::write_clipboard(args["text"].as_str().ok_or("Missing text")?)?;
            Ok(json!(true))
        }
        "learning" => service.store.lock().unwrap().learning(),
        "forget" => {
            service.store.lock().unwrap().forget(
                args["kind"].as_str().unwrap_or(""),
                args["key"].as_str().unwrap_or(""),
            )?;
            Ok(json!(true))
        }
        "export" => {
            let threshold = service.config.lock().unwrap()["teacher_min_confidence"]
                .as_f64()
                .unwrap_or(0.7);
            Ok(json!(service.store.lock().unwrap().export(threshold)?))
        }
        "import" => {
            let source = Path::new(args["path"].as_str().ok_or("Source directory required")?);
            Ok(json!(service.store.lock().unwrap().import(source)?))
        }
        "export_config" => Ok(json!(toml::to_string_pretty(
            &*service.config.lock().unwrap()
        )
        .map_err(crate::config::err)?)),
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
            service.update(cfg)?;
            Ok(json!(true))
        }
        "install" => {
            if service.learning_busy.load(Ordering::SeqCst) {
                return Err("Pause learning before installing models".into());
            }
            let id = args["id"].as_str().ok_or("Model ID required")?.to_owned();
            if id != "training" {
                crate::models::spec(&id)?;
            }
            if service.downloading.swap(true, Ordering::SeqCst) {
                return Err(
                    "One model download at a time; cancel or wait for the current download".into(),
                );
            }
            service.cancel.store(false, Ordering::SeqCst);
            let service = service.clone();
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
            service.cancel_work();
            Ok(json!(true))
        }
        "load" => {
            service.preload()?;
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
            let dest = crate::models::path(&service.root, id)?;
            let tmp = dest.with_extension("partial");
            std::fs::copy(source, &tmp).map_err(crate::config::err)?;
            crate::models::verify(&tmp, hash, &std::sync::atomic::AtomicBool::new(false))?;
            std::fs::rename(tmp, dest).map_err(crate::config::err)?;
            Ok(json!(true))
        }
        "train" => {
            crate::learning::start_training(service.clone())?;
            Ok(json!(true))
        }
        "rollback" => {
            crate::learning::rollback(service)?;
            Ok(json!(true))
        }
        "autostart" => {
            super::autostart::set_enabled(args["enabled"] == true)?;
            Ok(json!(true))
        }
        _ => Err("Unknown command".into()),
    }
}
fn id(v: &Value) -> Result<i64> {
    v["id"].as_i64().ok_or("Sample ID required".into())
}
