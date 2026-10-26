use crate::{
    config::{self, Result},
    models,
    store::Store,
    worker::{Worker, WorkerSlot},
};
use serde_json::{json, Value};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};
pub struct Service {
    _lock: std::fs::File,
    pub root: PathBuf,
    pub native: PathBuf,
    pub config: Mutex<Value>,
    pub store: Mutex<Store>,
    pub state: Mutex<Value>,
    pub worker: WorkerSlot,
    pub pid: Arc<AtomicU32>,
    pub cancel: Arc<AtomicBool>,
    pub stopping: AtomicBool,
    pub recording: AtomicBool,
    pub generation: AtomicU64,
    pub last_activity: Mutex<Instant>,
    pub downloading: AtomicBool,
    pub training_pid: AtomicU32,
    pub learning_busy: AtomicBool,
}
impl Service {
    pub fn new(root: PathBuf, native: PathBuf) -> Result<Arc<Self>> {
        let lock = config::lock(&root)?;
        // Only this candidate's temporary capture files; no existing Speechify paths.
        for entry in std::fs::read_dir(&root)
            .map_err(|e| e.to_string())?
            .flatten()
        {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("pending-")
                && name.ends_with(".wav")
                && entry.file_type().is_ok_and(|t| t.is_file())
            {
                let _ = std::fs::remove_file(entry.path());
            }
        }
        let cfg = config::load(&root)?;
        config::save(&root, &cfg)?;
        let store = Store::open(&root)?;
        Ok(Arc::new(Self {
            _lock: lock,
            root,
            native,
            config: Mutex::new(cfg),
            store: Mutex::new(store),
            state: Mutex::new(
                json!({"stage":"Idle","ready":false,"level":0,"recent":"","device":"Not loaded","error":null,"download":null}),
            ),
            worker: Arc::new(Mutex::new(None)),
            pid: Arc::new(AtomicU32::new(0)),
            cancel: Arc::new(AtomicBool::new(false)),
            stopping: AtomicBool::new(false),
            recording: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            last_activity: Mutex::new(Instant::now()),
            downloading: AtomicBool::new(false),
            training_pid: AtomicU32::new(0),
            learning_busy: AtomicBool::new(false),
        }))
    }
    pub fn set_stage(&self, stage: &str) {
        self.state.lock().unwrap()["stage"] = json!(stage);
    }
    pub fn error(&self, e: impl ToString) {
        let mut state = self.state.lock().unwrap();
        state["error"] = json!(e.to_string());
        state["stage"] = json!("Error");
    }
    pub fn log(&self, event: &str, values: Value) {
        let p = self.root.join("logs/timings.jsonl");
        if std::fs::metadata(&p).is_ok_and(|m| m.len() > 2_000_000) {
            let old = self.root.join("logs/timings.1.jsonl");
            let _ = std::fs::remove_file(&old);
            let _ = std::fs::rename(&p, old);
        }
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
        {
            let _ = writeln!(
                f,
                "{}",
                json!({"ts":crate::store::now(),"event":event,"values":values})
            );
        }
    }
    pub fn ensure<'a>(&self, slot: &'a mut Option<Worker>, cfg: &Value) -> Result<&'a mut Worker> {
        if self.stopping.load(Ordering::SeqCst) {
            return Err("Service is stopping".into());
        }
        let id = cfg["model"].as_str().ok_or("Model missing")?;
        let key = format!(
            "{}:{}:{}:{}",
            id, cfg["device"], cfg["onnx_threads"], cfg["compute_type"]
        );
        if slot.as_mut().is_none_or(|w| w.key != key || !w.alive()) {
            slot.take();
            self.set_stage("Loading model");
            let model = models::path(&self.root, id)?;
            let python_needed =
                id.starts_with("ft:") || models::spec(id).is_ok_and(|s| s["gguf"].is_null());
            let model = if python_needed && !id.starts_with("ft:") {
                self.root.join("models").join(id)
            } else {
                model
            };
            if !model.exists() {
                return Err("Install the selected model in Models before dictating".into());
            }
            if !python_needed {
                let spec = models::spec(id)?;
                models::verify(
                    &model,
                    spec["gguf"]["sha256"].as_str().unwrap(),
                    &AtomicBool::new(false),
                )?;
            }
            let py = self
                .root
                .join("environments")
                .join(if id.starts_with("ft:") || id.starts_with("distil") {
                    "whisper"
                } else {
                    "transformers"
                })
                .join(if cfg!(windows) {
                    "Scripts/python.exe"
                } else {
                    "bin/python"
                });
            if python_needed && !py.exists() {
                return Err(
                    "Optional Python model environment is not installed; see build instructions"
                        .into(),
                );
            }
            let w = Worker::start(
                &self.native,
                &model,
                cfg["device"].as_str().unwrap_or("cpu"),
                cfg["onnx_threads"].as_i64().unwrap_or(4) as i32,
                key,
                self.pid.clone(),
                if python_needed { Some(&py) } else { None },
                cfg,
            )?;
            self.log(
                "model_ready",
                json!({"model":id,"device":w.device,"load_ms":w.load_ms}),
            );
            {
                let mut st = self.state.lock().unwrap();
                st["ready"] = json!(true);
                st["device"] = json!(w.device);
            }
            *slot = Some(w);
        }
        Ok(slot.as_mut().unwrap())
    }
    pub fn preload(&self) -> Result<()> {
        let cfg = self.config.lock().unwrap().clone();
        let mut slot = self.worker.lock().unwrap();
        self.ensure(&mut slot, &cfg)?;
        self.set_stage("Ready");
        Ok(())
    }
    pub fn infer_path(&self, path: &Path, save: bool) -> Result<Value> {
        let cfg = self.config.lock().unwrap().clone();
        let generation = self.generation.load(Ordering::SeqCst);
        let prompt = self.store.lock().unwrap().prompt()?;
        let mut slot = self.worker.lock().unwrap();
        let mut result = {
            let worker = self.ensure(&mut slot, &cfg)?;
            self.set_stage("Transcribing");
            worker.run(path, cfg["language"].as_str().unwrap_or("en"), &prompt)
        };
        // A native crash gets one clean CPU retry; cancellation must not restart it.
        if result.is_err()
            && self.generation.load(Ordering::SeqCst) == generation
            && !self.stopping.load(Ordering::SeqCst)
        {
            slot.take();
            let mut cpu = cfg.clone();
            cpu["device"] = json!("cpu");
            result = self.ensure(&mut slot, &cpu)?.run(
                path,
                cfg["language"].as_str().unwrap_or("en"),
                &prompt,
            );
        }
        let mut result = result?;
        if self.generation.load(Ordering::SeqCst) != generation {
            return Err("Cancelled".into());
        }
        let raw = result["text"].as_str().unwrap_or("").to_owned();
        let mut text = self
            .store
            .lock()
            .unwrap()
            .apply(&raw, cfg["autocorrect_after"].as_u64().unwrap_or(2))?;
        if !cfg["llm_url"].as_str().unwrap_or("").is_empty() && !text.is_empty() {
            match cleanup_text(cfg["llm_url"].as_str().unwrap(), &cfg["llm_model"], &text) {
                Ok(cleaned) => text = cleaned,
                Err(_) => {
                    // Optional remote cleanup must not discard a valid local dictation.
                    result["cleanup_warning"] = json!("Cleanup unavailable; using local transcription");
                    self.log("cleanup_unavailable", json!({}));
                }
            }
        }
        if self.generation.load(Ordering::SeqCst) != generation {
            return Err("Cancelled".into());
        }
        if save && !text.trim().is_empty() {
            let audio = crate::audio::load(path)?;
            result["sample_id"] = json!(self.store.lock().unwrap().record(
                &audio,
                &raw,
                &text,
                cfg["keep_all_audio"].as_bool().unwrap_or(true)
            )?);
        }
        result["text"] = json!(text);
        {
            let mut state = self.state.lock().unwrap();
            state["recent"] = result["text"].clone();
            state["device"] = result["device"].clone();
            state["stage"] = json!("Ready");
            state["error"] = result["cleanup_warning"].clone();
        }
        self.log("inference",json!({"model":cfg["model"],"device":result["device"],"inference_ms":result["inference_ms"]}));
        *self.last_activity.lock().unwrap() = Instant::now();
        Ok(result)
    }
    pub fn cancel_work(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.cancel.store(true, Ordering::SeqCst);
        crate::worker::terminate(self.pid.load(Ordering::SeqCst));
        crate::worker::terminate(self.training_pid.load(Ordering::SeqCst));
        self.state.lock().unwrap()["ready"] = json!(false);
        self.set_stage("Cancelled");
    }
    pub fn shutdown(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        self.cancel_work();
    }
    pub fn update(&self, cfg: Value) -> Result<()> {
        config::validate(&cfg)?;
        if self.recording.load(Ordering::SeqCst) {
            return Err("Finish recording before applying settings".into());
        }
        let old = self.config.lock().unwrap().clone();
        config::save(&self.root, &cfg)?;
        let reload = ["model", "device", "onnx_threads", "compute_type"]
            .iter()
            .any(|k| old[*k] != cfg[*k]);
        *self.config.lock().unwrap() = cfg;
        if reload {
            self.cancel_work();
            self.worker.lock().unwrap().take();
            if let Err(e) = self.preload() {
                config::save(&self.root, &old)?;
                *self.config.lock().unwrap() = old;
                let _ = self.preload();
                return Err(format!("Selection rolled back: {e}"));
            }
        }
        Ok(())
    }
    pub fn snapshot(&self) -> Value {
        json!({"state":self.state.lock().unwrap().clone(),"config":self.config.lock().unwrap().clone(),"models":models::list(&self.root),"data_dir":self.root,"diagnostics":diagnostics()})
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        self.shutdown();
    }
}
pub fn diagnostics() -> Value {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var("XDG_SESSION_TYPE").is_ok_and(|s| s == "wayland");
    json!({"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"session":if cfg!(target_os="linux"){if wayland{"Wayland"}else{"X11"}}else{"native"},"guidance":if cfg!(target_os="macos"){ "Grant Microphone, Accessibility and Input Monitoring to Speechify Tauri Candidate in System Settings. Restart after grants. Use a stable signing identity." }else if cfg!(windows){ "Enable microphone access for desktop applications. Windows UIPI blocks pasting into elevated applications from a non-elevated candidate." }else if wayland{ "Wayland recording requires explicitly granted read access to /dev/input/by-id/*-event-kbd. Safe target discovery/paste is unavailable: transcripts are saved for manual copy (wl-clipboard required). No input privileges or uinput devices are created automatically. Use X11 for automatic paste." }else{ "X11 requires DISPLAY, libX11, xclip and xdotool. Microphone permission is controlled by PipeWire/PulseAudio/ALSA. Clipboard restoration preserves text only, not images or rich formats." }})
}

fn cleanup_text(url: &str, model: &Value, text: &str) -> Result<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15)).build().map_err(|e|e.to_string())?;
    let response: Value = client.post(url).json(&json!({"model":model,"temperature":0,
        "messages":[{"role":"system","content":"Clean dictation punctuation and grammar. Return only the corrected text. Preserve meaning."},
                    {"role":"user","content":text}]}))
        .send().and_then(|r|r.error_for_status()).and_then(|r|r.json()).map_err(|e|e.to_string())?;
    response["choices"][0]["message"]["content"].as_str()
        .filter(|s| !s.trim().is_empty()).map(str::to_owned).ok_or("LLM returned no text".into())
}
#[cfg(test)]
mod cleanup_tests {
    use super::*;
    #[test]
    fn unavailable_cleanup_is_an_error_not_empty_output() {
        assert!(cleanup_text("invalid://cleanup", &json!("model"), "Keep my words.").is_err());
    }
}
