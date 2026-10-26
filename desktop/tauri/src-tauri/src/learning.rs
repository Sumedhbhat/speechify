use crate::{
    config::{owned, Result},
    service::Service,
    store::{label, now},
};
use serde_json::{json, Value};
use std::{
    path::Path,
    process::{Command, Stdio},
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};
pub fn mains() -> bool {
    #[cfg(target_os = "linux")]
    {
        let Ok(entries) = std::fs::read_dir("/sys/class/power_supply") else {
            return false;
        };
        let mut battery = false;
        for d in entries.flatten() {
            let p = d.path();
            let kind = std::fs::read_to_string(p.join("type")).unwrap_or_default();
            if kind.trim() == "Battery" {
                battery = true;
                let status = std::fs::read_to_string(p.join("status")).unwrap_or_default();
                if ["Charging", "Full"].contains(&status.trim()) {
                    return true;
                }
            }
            if std::fs::read_to_string(p.join("online")).is_ok_and(|s| s.trim() == "1") {
                return true;
            }
        }
        !battery
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("/usr/bin/pmset")
            .args(["-g", "batt"])
            .output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("AC Power"))
    }
    #[cfg(windows)]
    {
        #[repr(C)]
        struct Power {
            ac: u8,
            flag: u8,
            percent: u8,
            reserved: u8,
            life: u32,
            full: u32,
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn GetSystemPowerStatus(p: *mut Power) -> i32;
        }
        let mut p = Power {
            ac: 255,
            flag: 0,
            percent: 0,
            reserved: 0,
            life: 0,
            full: 0,
        };
        unsafe { GetSystemPowerStatus(&mut p) != 0 && p.ac == 1 }
    }
}
pub fn scheduler(s: Arc<Service>) {
    std::thread::spawn(move || {
        let mut retry = Instant::now();
        while !s.stopping.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_secs(5));
            if s.stopping.load(Ordering::SeqCst) {
                break;
            }
            let cfg = s.config.lock().unwrap().clone();
            let idle = s.last_activity.lock().unwrap().elapsed().as_secs_f64()
                >= cfg["idle_minutes"].as_f64().unwrap_or(5.) * 60.;
            if !mains() {
                if s.learning_busy.load(Ordering::SeqCst) {
                    s.cancel_work();
                }
                continue;
            }
            if !idle
                || s.recording.load(Ordering::SeqCst)
                || cfg["learn_auto"] != true
                || Instant::now() < retry
                || s.learning_busy.load(Ordering::SeqCst)
            {
                continue;
            }
            if let Err(e) = teacher(&s, &cfg) {
                let _ = s.store.lock().unwrap().state("teacher_status", &json!(e));
                retry = Instant::now() + Duration::from_secs(3600);
            }
            let counts = s
                .store
                .lock()
                .unwrap()
                .history("", "")
                .unwrap_or_default()
                .iter()
                .filter(|r| {
                    label(r, cfg["teacher_min_confidence"].as_f64().unwrap_or(0.7)).is_some()
                })
                .count();
            let previous = s
                .store
                .lock()
                .unwrap()
                .get_state("last_training")
                .unwrap_or(json!({"count":0,"ts":now()}));
            if due(
                counts,
                previous["count"].as_u64().unwrap_or(0) as usize,
                previous["ts"].as_f64().unwrap_or(now()),
                now(),
                &cfg,
            ) {
                let _ = start_training(s.clone());
            }
        }
    });
}
pub fn due(total: usize, previous: usize, ts: f64, now: f64, cfg: &Value) -> bool {
    total >= 50
        && total > previous
        && (total - previous >= cfg["retrain_after_labels"].as_u64().unwrap_or(300) as usize
            || now - ts >= cfg["retrain_after_days"].as_f64().unwrap_or(7.) * 86400.)
}
fn teacher(s: &Arc<Service>, cfg: &Value) -> Result<()> {
    let mut teacher = cfg["teacher"].as_str().unwrap_or("off").to_owned();
    if teacher == "off" {
        return Ok(());
    }
    if cfg["teacher_on_cpu"] != true && cfg["device"] == "cpu" {
        return Ok(());
    }
    if teacher == "auto" {
        teacher = "granite-speech-4.1-2b".into();
    }
    if !crate::models::path(&s.root, &teacher)?.exists() {
        return Err(format!(
            "Install teacher {teacher} in Models; no automatic large download"
        ));
    }
    let rows = s.store.lock().unwrap().history("", "")?;
    let row = rows.into_iter().find(|r| {
        r["audio"].is_string()
            && r["excluded"] == 0
            && r["corrected"].is_null()
            && r["teacher_model"].is_null()
    });
    let Some(row) = row else {
        return Ok(());
    };
    if s.learning_busy.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let result: Result<()> = (|| {
        let id = row["id"].as_i64().unwrap();
        let path = s.store.lock().unwrap().audio_path(id)?;
        let mut teacher_cfg = cfg.clone();
        teacher_cfg["model"] = json!(teacher);
        let generation = s.generation.load(Ordering::SeqCst);
        let mut slot = s.worker.lock().unwrap();
        let w = s.ensure(&mut slot, &teacher_cfg)?;
        if w.device.starts_with("cpu") && cfg["teacher_on_cpu"] != true {
            slot.take();
            return Err("Teacher paused: CPU requires explicit opt-in".into());
        }
        s.store
            .lock()
            .unwrap()
            .state("teacher_status", &json!(format!("Labelling sample {id}")))?;
        let result = w.run(&path, cfg["language"].as_str().unwrap_or("en"), "")?;
        if s.generation.load(Ordering::SeqCst) != generation {
            return Err("Teacher preempted".into());
        }
        let text = result["text"].as_str().unwrap_or("");
        let audio = crate::audio::load(&path)?;
        let (confidence, note) = judge(text, audio.len() as f64 / 16000.);
        s.store.lock().unwrap().db.execute("UPDATE samples SET teacher_text=?,teacher_model=?,teacher_confidence=?,teacher_ts=?,teacher_note=? WHERE id=?",rusqlite::params![text,teacher,confidence,now(),note,id]).map_err(|e|e.to_string())?;
        s.store.lock().unwrap().state(
            "teacher_status",
            &json!(format!("Labelled sample {id}; confidence {confidence:.2}")),
        )?;
        slot.take();
        Ok(())
    })();
    s.learning_busy.store(false, Ordering::SeqCst);
    let _ = s.preload();
    result
}
pub fn judge(text: &str, seconds: f64) -> (f64, Option<String>) {
    let words: Vec<_> = text.split_whitespace().collect();
    if words.is_empty() {
        return (0., Some("empty".into()));
    }
    if words.len() as f64 > seconds * 6. + 3. {
        return (0., Some("too long for clip".into()));
    }
    for n in 1..=4 {
        for i in 0..words.len() {
            let count = if n == 1 { 5 } else { 3 };
            if i + n * count <= words.len()
                && (1..count).all(|k| words[i..i + n] == words[i + k * n..i + (k + 1) * n])
            {
                return (0., Some("repeated phrase".into()));
            }
        }
    }
    let rate = words.len() as f64 / seconds.max(0.1);
    let confidence = (0.8 - (rate - 4.5).max(0.) * 0.15 - (1. - rate).max(0.) * 0.5).clamp(0., 0.8);
    (confidence, None)
}
pub fn start_training(s: Arc<Service>) -> Result<()> {
    if s.learning_busy.swap(true, Ordering::SeqCst) {
        return Err("Learning job already running".into());
    }
    let cfg = s.config.lock().unwrap().clone();
    let selected = cfg["model"].as_str().unwrap_or("");
    if !["whisper-large-v3-turbo", "whisper-large-v3"].contains(&selected)
        && !selected.starts_with("ft:whisper-ft-")
    {
        s.learning_busy.store(false, Ordering::SeqCst);
        return Err("Select Whisper large-v3-turbo, Whisper large-v3, or a trained Whisper version before fine-tuning".into());
    }
    let py = s.root.join("environments/training").join(if cfg!(windows) {
        "Scripts/python.exe"
    } else {
        "bin/python"
    });
    if !py.exists() {
        s.learning_busy.store(false, Ordering::SeqCst);
        return Err("Install the optional training environment first. Training needs NVIDIA CUDA and at least 10 GB VRAM.".into());
    }
    if !mains() {
        s.learning_busy.store(false, Ordering::SeqCst);
        return Err("Training pauses on battery; connect mains power".into());
    }
    std::thread::spawn(move || {
        let result: Result<()> = (|| {
            s.cancel_work();
            s.worker.lock().unwrap().take();
            s.store.lock().unwrap().state(
                "training_status",
                &json!("Running held-out and LibriSpeech gates"),
            )?;
            let report = s.root.join("training-report.json");
            let _ = std::fs::remove_file(&report);
            let base = if cfg["model"] == "whisper-large-v3" {
                "whisper-large-v3"
            } else {
                "whisper-large-v3-turbo"
            };
            let current = if let Some(id) = cfg["model"].as_str().filter(|x| x.starts_with("ft:")) {
                let path = crate::models::path(&s.root, id)?;
                let stamp = path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .replace("whisper-ft-", "hf-");
                owned(&s.root.join("models"), &stamp)?
                    .to_string_lossy()
                    .into_owned()
            } else {
                format!("openai/{base}")
            };
            let mut cmd = Command::new(py);
            cmd.arg(
                s.native
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
            .env("SPEECHIFY_TAURI_TRAINING_HOME", &s.root)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                cmd.process_group(0);
                unsafe {
                    cmd.pre_exec(|| {
                        #[cfg(target_os = "linux")]
                        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                        Ok(())
                    });
                }
            }
            let mut child = cmd.spawn().map_err(|e| e.to_string())?;
            s.training_pid.store(child.id(), Ordering::SeqCst);
            let status = child.wait().map_err(|e| e.to_string())?;
            s.training_pid.store(0, Ordering::SeqCst);
            if !status.success() {
                return Err(
                    "Training failed or was preempted; previous model remains active".into(),
                );
            }
            let result: Value =
                serde_json::from_str(&std::fs::read_to_string(&report).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            s.store.lock().unwrap().state("training_status", &result)?;
            if !gates(&result) {
                return Ok(());
            }
            let path = Path::new(result["model"].as_str().ok_or("Missing trained model")?);
            let name = path
                .file_name()
                .ok_or("Invalid trained path")?
                .to_string_lossy()
                .into_owned();
            let safe = owned(&s.root.join("models"), &name)?;
            if safe.canonicalize().map_err(|e| e.to_string())?
                != path.canonicalize().map_err(|e| e.to_string())?
            {
                return Err("Trainer returned an external path".into());
            }
            let mut versions = s
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
            s.store.lock().unwrap().state("versions", &versions)?;
            let mut adopted = cfg.clone();
            adopted["model"] = json!(format!("ft:{name}"));
            s.update(adopted)?;
            prune(&s, cfg["keep_versions"].as_u64().unwrap_or(3) as usize)?;
            let count = s
                .store
                .lock()
                .unwrap()
                .history("", "")?
                .iter()
                .filter(|r| {
                    label(r, cfg["teacher_min_confidence"].as_f64().unwrap_or(0.7)).is_some()
                })
                .count();
            s.store
                .lock()
                .unwrap()
                .state("last_training", &json!({"ts":now(),"count":count}))?;
            Ok(())
        })();
        if let Err(e) = result {
            let _ = s.store.lock().unwrap().state("training_status", &json!(e));
        }
        s.learning_busy.store(false, Ordering::SeqCst);
        let _ = s.preload();
    });
    Ok(())
}
pub fn gates(v: &Value) -> bool {
    v["status"] == "accepted" && v["gate1"]["passed"] == true && v["gate2"]["passed"] == true
}
fn prune(s: &Service, keep: usize) -> Result<()> {
    let active = s.config.lock().unwrap()["model"]
        .as_str()
        .unwrap_or("")
        .trim_start_matches("ft:")
        .to_owned();
    let mut versions = s.store.lock().unwrap().get_state("versions")?;
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
                let candidate = owned(&s.root.join("models"), &name)?;
                if path.exists()
                    && path.canonicalize().map_err(|e| e.to_string())?
                        == candidate.canonicalize().map_err(|e| e.to_string())?
                {
                    std::fs::remove_dir_all(candidate).map_err(|e| e.to_string())?;
                }
            }
        }
    }
    s.store.lock().unwrap().state("versions", &versions)
}
pub fn rollback(s: &Service) -> Result<()> {
    let versions = s.store.lock().unwrap().get_state("versions")?;
    let list = versions.as_array().ok_or("No trained versions")?;
    let mut cfg = s.config.lock().unwrap().clone();
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
    s.update(cfg)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn both_gates_required() {
        assert!(!gates(
            &json!({"status":"accepted","gate1":{"passed":true}})
        ));
        assert!(gates(
            &json!({"status":"accepted","gate1":{"passed":true},"gate2":{"passed":true}})
        ));
    }
    #[test]
    fn hallucination_rejected() {
        assert_eq!(judge("yes yes yes yes yes", 3.).0, 0.);
    }
}
