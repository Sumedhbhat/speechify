//! Run training, adopt only accepted gates, retain versions and roll back.
use super::mains;
use crate::config::owned;
use crate::{
    config::Result,
    service::Service,
    store::{label, now},
};
use serde_json::{json, Value};
use std::{
    path::Path,
    process::{Command, Stdio},
    sync::{atomic::Ordering, Arc},
};

pub fn start_training(service: Arc<Service>) -> Result<()> {
    start_training_on_power(service, mains())
}
pub(super) fn start_training_on_power(service: Arc<Service>, on_mains: bool) -> Result<()> {
    if service.downloading.load(Ordering::SeqCst) || service.recording.load(Ordering::SeqCst) {
        return Err("Finish installation and recording before training".into());
    }
    if service.learning_busy.swap(true, Ordering::SeqCst) {
        return Err("Learning job already running".into());
    }
    let cfg = service.config.lock().unwrap().clone();
    let selected = cfg["model"].as_str().unwrap_or("");
    if !["whisper-large-v3-turbo", "whisper-large-v3"].contains(&selected)
        && !selected.starts_with("ft:whisper-ft-")
    {
        service.learning_busy.store(false, Ordering::SeqCst);
        return Err("Select Whisper large-v3-turbo, Whisper large-v3, or a trained Whisper version before fine-tuning".into());
    }
    let py = service
        .root
        .join("environments/training")
        .join(if cfg!(windows) {
            "Scripts/python.exe"
        } else {
            "bin/python"
        });
    if !py.exists()
        || !std::fs::read_to_string(service.root.join("environments/training/candidate-ready"))
            .is_ok_and(|receipt| receipt.trim() == "cuda-cu126-v1")
    {
        service.learning_busy.store(false, Ordering::SeqCst);
        return Err("Install the optional training environment first. Training needs NVIDIA CUDA and at least 10 GB VRAM.".into());
    }
    if !on_mains {
        service.learning_busy.store(false, Ordering::SeqCst);
        return Err("Training pauses on battery; connect mains power".into());
    }
    std::thread::spawn(move || {
        let count = service
            .store
            .lock()
            .unwrap()
            .history("", "")
            .unwrap_or_default()
            .iter()
            .filter(|r| label(r, cfg["teacher_min_confidence"].as_f64().unwrap_or(0.7)).is_some())
            .count();
        let result: Result<()> = (|| {
            service.cancel_work();
            service.worker.lock().unwrap().take();
            service.store.lock().unwrap().state(
                "training_status",
                &json!("Running held-out and LibriSpeech gates"),
            )?;
            let report = service.root.join("training-report.json");
            let _ = std::fs::remove_file(&report);
            let base = if cfg["model"] == "whisper-large-v3" {
                "whisper-large-v3"
            } else {
                "whisper-large-v3-turbo"
            };
            let current = if let Some(id) = cfg["model"].as_str().filter(|x| x.starts_with("ft:")) {
                let path = crate::models::path(&service.root, id)?;
                let stamp = path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .replace("whisper-ft-", "hf-");
                owned(&service.root.join("models"), &stamp)?
                    .to_string_lossy()
                    .into_owned()
            } else {
                format!("openai/{base}")
            };
            let mut cmd = Command::new(py);
            cmd.arg(
                service
                    .native
                    .parent()
                    .unwrap()
                    .join("training/scripts/finetune.py"),
            )
            .args([
                "--auto",
                "--base",
                &format!("openai/{base}"),
                "--base-id",
                base,
                "--current",
                &current,
                "--report",
            ])
            .arg(&report)
            .env("SPEECHIFY_TAURI_TRAINING_HOME", &service.root)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
            let (mut child, _owner) = crate::child_owner::spawn(&mut cmd)?;
            service.training_pid.store(child.id(), Ordering::SeqCst);
            let status = child.wait().map_err(crate::config::err)?;
            service.training_pid.store(0, Ordering::SeqCst);
            if !status.success() {
                return Err(
                    "Training failed or was preempted; previous model remains active".into(),
                );
            }
            let result: Value = serde_json::from_str(
                &std::fs::read_to_string(&report).map_err(crate::config::err)?,
            )
            .map_err(crate::config::err)?;
            service
                .store
                .lock()
                .unwrap()
                .state("training_status", &result)?;
            if !gates(&result) {
                return Ok(());
            }
            let path = Path::new(result["model"].as_str().ok_or("Missing trained model")?);
            let name = path
                .file_name()
                .ok_or("Invalid trained path")?
                .to_string_lossy()
                .into_owned();
            let safe = owned(&service.root.join("models"), &name)?;
            if safe.canonicalize().map_err(crate::config::err)?
                != path.canonicalize().map_err(crate::config::err)?
            {
                return Err("Trainer returned an external path".into());
            }
            let mut versions = service
                .store
                .lock()
                .unwrap()
                .get_state("versions")
                .unwrap_or(json!([]));
            let entry = json!({"name":name,"path":safe,"hf":result["hf"],"base":base,"ts":now(),"gate1":result["gate1"],"gate2":result["gate2"]});
            versions
                .as_array_mut()
                .ok_or("Invalid versions state")?
                .push(entry);
            service.store.lock().unwrap().state("versions", &versions)?;
            let mut adopted = cfg.clone();
            adopted["model"] = json!(format!("ft:{name}"));
            service.update(adopted)?;
            prune(
                &service,
                cfg["keep_versions"].as_u64().unwrap_or(3) as usize,
            )?;
            Ok(())
        })();
        if let Err(e) = result {
            let _ = service
                .store
                .lock()
                .unwrap()
                .state("training_status", &json!(e));
        }
        // Failed, rejected and cancelled attempts also consume this scheduling batch.
        let _ = service
            .store
            .lock()
            .unwrap()
            .state("last_training", &json!({"ts":now(),"count":count}));
        service.learning_busy.store(false, Ordering::SeqCst);
        let _ = service.preload();
    });
    Ok(())
}
pub fn gates(v: &Value) -> bool {
    v["status"] == "accepted" && v["gate1"]["passed"] == true && v["gate2"]["passed"] == true
}
pub(super) fn prune(service: &Service, keep: usize) -> Result<()> {
    let active = service.config.lock().unwrap()["model"]
        .as_str()
        .unwrap_or("")
        .trim_start_matches("ft:")
        .to_owned();
    let mut versions = service.store.lock().unwrap().get_state("versions")?;
    let entries = versions.as_array_mut().ok_or("Invalid version ledger")?;
    while entries.len() > keep.max(1) {
        let Some(i) = entries.iter().position(|v| v["name"] != active) else {
            break;
        };
        let v = entries.remove(i);
        for key in ["path", "hf"] {
            if let Some(p) = v[key].as_str() {
                let path = Path::new(p);
                let name = path
                    .file_name()
                    .ok_or("Unsafe version path")?
                    .to_string_lossy();
                let candidate = owned(&service.root.join("models"), &name)?;
                if path.exists()
                    && path.canonicalize().map_err(crate::config::err)?
                        == candidate.canonicalize().map_err(crate::config::err)?
                {
                    std::fs::remove_dir_all(candidate).map_err(crate::config::err)?;
                }
            }
        }
    }
    service.store.lock().unwrap().state("versions", &versions)
}
pub fn rollback(service: &Service) -> Result<()> {
    let versions = service.store.lock().unwrap().get_state("versions")?;
    let list = versions.as_array().ok_or("No trained versions")?;
    let mut cfg = service.config.lock().unwrap().clone();
    let active = cfg["model"]
        .as_str()
        .unwrap_or("")
        .trim_start_matches("ft:");
    let i = list
        .iter()
        .position(|v| v["name"] == active)
        .ok_or("Current model is not an adopted training version")?;
    cfg["model"] = if i > 0 {
        json!(format!("ft:{}", list[i - 1]["name"].as_str().unwrap()))
    } else {
        list[i]["base"].clone()
    };
    service.update(cfg)
}
