//! Select a consented teacher and label an eligible recording.
use crate::{config::Result, service::Service, store::now};
use serde_json::{json, Value};
use std::{
    process::Command,
    sync::{atomic::Ordering, Arc},
};

pub(super) fn teacher(service: &Arc<Service>, cfg: &Value) -> Result<()> {
    let mut teacher = cfg["teacher"].as_str().unwrap_or("off").to_owned();
    if teacher == "off" {
        return Ok(());
    }
    let memory = if cfg["device"] == "cpu" {
        None
    } else {
        gpu_memory()
    };
    let (selected, device) = choose_teacher(&teacher, memory, cfg["teacher_on_cpu"] == true)?;
    teacher = selected;
    if !crate::models::path(&service.root, &teacher)?.exists() {
        return Err(format!(
            "Install teacher {teacher} in Models; no automatic large download"
        ));
    }
    let rows = service.store.lock().unwrap().history("", "")?;
    let row = rows.into_iter().find(|r| {
        r["audio"].is_string()
            && r["excluded"] == 0
            && r["corrected"].is_null()
            && r["teacher_model"].is_null()
    });
    let Some(row) = row else {
        return Ok(());
    };
    if service.learning_busy.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let result: Result<()> = (|| {
        let id = row["id"].as_i64().unwrap();
        let path = service.store.lock().unwrap().audio_path(id)?;
        let mut teacher_cfg = cfg.clone();
        teacher_cfg["model"] = json!(teacher);
        teacher_cfg["device"] = json!(device);
        teacher_cfg["beam_size"] = json!(5);
        let generation = service.generation.load(Ordering::SeqCst);
        let mut slot = service.worker.lock().unwrap();
        let w = service.ensure(&mut slot, &teacher_cfg)?;
        if w.device.starts_with("cpu") && cfg["teacher_on_cpu"] != true {
            slot.take();
            return Err("Teacher paused: CPU requires explicit opt-in".into());
        }
        service
            .store
            .lock()
            .unwrap()
            .state("teacher_status", &json!(format!("Labelling sample {id}")))?;
        let result = w.run(
            &path,
            cfg["language"].as_str().unwrap_or("en"),
            "",
            crate::worker::transcription_timeout(&path)?,
        )?;
        if service.generation.load(Ordering::SeqCst) != generation {
            return Err("Teacher preempted".into());
        }
        let text = result["text"].as_str().unwrap_or("");
        let audio = crate::audio::load(&path)?;
        let (confidence, note) = judge(text, audio.len() as f64 / 16000.);
        service
            .store
            .lock()
            .unwrap()
            .db
            .execute(
                "UPDATE samples SET teacher_text = ?, teacher_model = ?,
                    teacher_confidence = ?, teacher_ts = ?, teacher_note = ?
             WHERE id = ?",
                rusqlite::params![text, teacher, confidence, now(), note, id],
            )
            .map_err(crate::config::err)?;
        service.store.lock().unwrap().state(
            "teacher_status",
            &json!(format!("Labelled sample {id}; confidence {confidence:.2}")),
        )?;
        slot.take();
        Ok(())
    })();
    service.learning_busy.store(false, Ordering::SeqCst);
    let _ = service.preload();
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
// Match the reference's NVIDIA memory/headroom policy; unknown hardware never implies permission for CPU.
pub(super) fn gpu_memory() -> Option<f64> {
    let output = Command::new("nvidia-smi")
        .args(["--query-gpu=memory.total", "--format=csv,noheader,nounits"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()?
        .lines()
        .next()?
        .trim()
        .parse::<f64>()
        .ok()
        .map(|m| m / 1024.)
}
pub(super) fn choose_teacher(
    setting: &str,
    memory: Option<f64>,
    cpu: bool,
) -> Result<(String, &'static str)> {
    let ids = if setting == "auto" {
        vec!["granite-speech-4.1-2b", "qwen3-asr-1.7b"]
    } else {
        vec![setting]
    };
    let candidates = ids
        .iter()
        .map(|id| crate::models::spec(id))
        .collect::<Result<Vec<_>>>()?;
    if let Some(gb) = memory {
        for model in &candidates {
            if model["vram_gb"].as_f64().unwrap_or(f64::INFINITY) + 1. <= gb {
                return Ok((model["id"].as_str().unwrap().into(), "cuda"));
            }
        }
    }
    if cpu {
        let smallest = candidates
            .iter()
            .min_by(|a, b| {
                a["vram_gb"]
                    .as_f64()
                    .unwrap_or(f64::INFINITY)
                    .total_cmp(&b["vram_gb"].as_f64().unwrap_or(f64::INFINITY))
            })
            .ok_or("No teacher")?;
        return Ok((smallest["id"].as_str().unwrap().into(), "cpu"));
    }
    Err("Teacher paused: no suitable NVIDIA GPU with 1 GB headroom; enable teacher_on_cpu to opt in".into())
}
