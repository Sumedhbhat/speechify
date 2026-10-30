//! Select, verify and supervise the active inference worker.
use super::Service;
use crate::{config::Result, models, worker::Worker};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};

impl Service {
    pub fn ensure<'a>(&self, slot: &'a mut Option<Worker>, cfg: &Value) -> Result<&'a mut Worker> {
        if self.stopping.load(Ordering::SeqCst) {
            return Err("Service is stopping".into());
        }
        let id = cfg["model"].as_str().ok_or("Model missing")?;
        if models::environment(id)
            .is_some_and(|family| self.installing_family.lock().unwrap().as_deref() == Some(family))
        {
            return Err(
                "This model's optional environment is being installed; retry when setup finishes"
                    .into(),
            );
        }
        let key = model_key(cfg);
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
}
pub(super) fn model_key(cfg: &Value) -> String {
    json!([
        cfg["model"],
        cfg["device"],
        cfg["onnx_threads"],
        cfg["compute_type"],
        cfg["language"],
        cfg["beam_size"]
    ])
    .to_string()
}
