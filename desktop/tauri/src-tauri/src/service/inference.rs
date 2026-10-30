//! Transcribe, apply corrections and persist successful dictations.
use super::Service;
use crate::config::Result;
use serde_json::{json, Value};
use std::{path::Path, sync::atomic::Ordering, time::Instant};

impl Service {
    pub fn infer_path(&self, path: &Path, save: bool) -> Result<Value> {
        let cfg = self.config.lock().unwrap().clone();
        let generation = self.generation.load(Ordering::SeqCst);
        let prompt = self.store.lock().unwrap().prompt()?;
        let timeout = crate::worker::transcription_timeout(path)?;
        let mut slot = self.worker.lock().unwrap();
        let mut result = {
            let worker = self.ensure(&mut slot, &cfg)?;
            self.set_stage("Transcribing");
            worker.run(
                path,
                cfg["language"].as_str().unwrap_or("en"),
                &prompt,
                timeout,
            )
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
                timeout,
            );
        }
        if self.generation.load(Ordering::SeqCst) != generation {
            return Err("Cancelled".into());
        }
        let mut result = result?;
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
                    result["cleanup_warning"] =
                        json!("Cleanup unavailable; using local transcription");
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
        self.log(
            "inference",
            json!({
                "model": cfg["model"],
                "device": result["device"],
                "inference_ms": result["inference_ms"],
            }),
        );
        *self.last_activity.lock().unwrap() = Instant::now();
        Ok(result)
    }
}
pub(super) fn cleanup_text(url: &str, model: &Value, text: &str) -> Result<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(crate::config::err)?;
    let response: Value = client
        .post(url)
        .json(&json!({
            "model": model,
            "temperature": 0,
            "messages": [
                {
                    "role": "system",
                    "content": "Clean dictation punctuation and grammar. Return only the corrected text. Preserve meaning.",
                },
                { "role": "user", "content": text },
            ],
        }))
        .send()
        .and_then(|response| response.error_for_status())
        .and_then(|response| response.json())
        .map_err(crate::config::err)?;
    response["choices"][0]["message"]["content"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
        .ok_or("LLM returned no text".into())
}
