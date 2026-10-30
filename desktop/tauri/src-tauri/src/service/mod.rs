//! Shared application state, configuration updates and cancellation lifecycle.
mod diagnostics;
mod inference;
mod model;

pub use diagnostics::diagnostics;
#[cfg(test)]
use inference::cleanup_text;
use model::model_key;

use crate::{
    config::{self, Result},
    models,
    store::Store,
    worker::WorkerSlot,
};
use serde_json::{json, Value};
use std::{
    io::Write,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};
#[cfg(test)]
#[path = "../../tests/unit/service.rs"]
mod behavior;
pub struct Service {
    _lock: std::fs::File,
    pub root: PathBuf,
    pub native: PathBuf,
    pub config: Mutex<Value>,
    pub store: Mutex<Store>,
    pub state: Mutex<Value>,
    pub worker: WorkerSlot,
    #[cfg(feature = "desktop")]
    pub capture_tx:
        Mutex<Option<std::sync::mpsc::SyncSender<crate::desktop::runtime::CaptureCommand>>>,
    #[cfg(feature = "desktop")]
    pub manual_started: Mutex<Option<Instant>>,
    #[cfg(feature = "desktop")]
    pub replacement: Mutex<Option<(i64, crate::replacement::Receipt)>>,
    pub pid: Arc<AtomicU32>,
    pub cancel: Arc<AtomicBool>,
    pub stopping: AtomicBool,
    pub recording: AtomicBool,
    pub generation: AtomicU64,
    pub last_activity: Mutex<Instant>,
    pub downloading: AtomicBool,
    pub training_pid: AtomicU32,
    pub install_pid: AtomicU32,
    pub installing_family: Mutex<Option<String>>,
    pub learning_busy: AtomicBool,
}
impl Service {
    pub fn new(root: PathBuf, native: PathBuf) -> Result<Arc<Self>> {
        let lock = config::lock(&root)?;
        // Only this candidate's temporary capture files; no existing Speechify paths.
        for entry in std::fs::read_dir(&root)
            .map_err(crate::config::err)?
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
            #[cfg(feature = "desktop")]
            capture_tx: Mutex::new(None),
            #[cfg(feature = "desktop")]
            manual_started: Mutex::new(None),
            #[cfg(feature = "desktop")]
            replacement: Mutex::new(None),
            pid: Arc::new(AtomicU32::new(0)),
            cancel: Arc::new(AtomicBool::new(false)),
            stopping: AtomicBool::new(false),
            recording: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            last_activity: Mutex::new(Instant::now()),
            downloading: AtomicBool::new(false),
            training_pid: AtomicU32::new(0),
            install_pid: AtomicU32::new(0),
            installing_family: Mutex::new(None),
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
    pub fn cancel_work(&self) {
        #[cfg(feature = "desktop")]
        {
            self.manual_started.lock().unwrap().take();
            self.recording.store(false, Ordering::SeqCst);
            if let Some(tx) = self.capture_tx.lock().unwrap().as_ref() {
                let _ = tx.try_send(crate::desktop::runtime::CaptureCommand {
                    action: crate::keys::Action::Cancel,
                    target: 0,
                    generation: 0,
                });
            }
        }
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.cancel.store(true, Ordering::SeqCst);
        crate::worker::terminate(self.pid.load(Ordering::SeqCst));
        crate::worker::terminate(self.training_pid.load(Ordering::SeqCst));
        crate::worker::terminate(self.install_pid.load(Ordering::SeqCst));
        self.state.lock().unwrap()["ready"] = json!(false);
        self.set_stage("Cancelled");
    }
    pub fn shutdown(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        self.cancel_work();
    }
    pub fn update(&self, cfg: Value) -> Result<()> {
        config::validate(&cfg)?;
        if self.downloading.load(Ordering::SeqCst) {
            return Err("Wait for installation or cancel it before changing settings".into());
        }
        if self.recording.load(Ordering::SeqCst) {
            return Err("Finish recording before applying settings".into());
        }
        let old = self.config.lock().unwrap().clone();
        config::save(&self.root, &cfg)?;
        let reload = model_key(&old) != model_key(&cfg);
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
        let mut snapshot = json!({
            "state": self.state.lock().unwrap().clone(),
            "config": self.config.lock().unwrap().clone(),
            "models": models::list(&self.root),
            "data_dir": self.root,
            "diagnostics": diagnostics(),
        });
        #[cfg(feature = "desktop")]
        {
            snapshot["manual_recording"] = json!(self.manual_started.lock().unwrap().is_some());
        }
        snapshot
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        self.shutdown();
    }
}
#[cfg(test)]
#[path = "../../tests/unit/service_cleanup.rs"]
mod cleanup_tests;

#[cfg(test)]
#[path = "../../tests/unit/service_reload.rs"]
mod reload_tests;
