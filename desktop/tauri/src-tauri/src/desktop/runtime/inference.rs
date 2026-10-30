//! Consume the bounded dictation queue and paste only into the captured target.
use super::Dictation;
use crate::{config::Result, platform, service::Service};
use serde_json::json;
use std::sync::{atomic::Ordering, mpsc::Receiver, Arc};

pub(super) fn run(service: Arc<Service>, rx: Receiver<Dictation>) {
    for Dictation {
        audio,
        target,
        generation,
    } in rx
    {
        if service.stopping.load(Ordering::SeqCst) {
            break;
        }
        if service.generation.load(Ordering::SeqCst) != generation {
            continue;
        }
        let path = service.root.join(format!(
            "pending-{}.wav",
            (crate::store::now() * 1e6) as u64
        ));
        let result: Result<()> = (|| {
            crate::audio::save(&path, &audio)?;
            let result = service.infer_path(&path, true)?;
            if service.generation.load(Ordering::SeqCst) != generation {
                return Err("Cancelled".into());
            }
            let text = result["text"].as_str().unwrap_or("");
            if !text.is_empty() {
                if target == 0 {
                    service.state.lock().unwrap()["error"] = json!("Transcript ready. Use Copy text to insert it; no safe external target was established.");
                    return Ok(());
                }
                let receipt = platform::paste(target, text)?;
                *service.replacement.lock().unwrap() = result["sample_id"].as_i64().zip(receipt);
                service.set_stage("Pasted");
                service.log("paste_ack", json!({"clock_ms":crate::store::now()*1000.}));
            }
            Ok(())
        })();
        let _ = std::fs::remove_file(path);
        if let Err(e) = result {
            if service.generation.load(Ordering::SeqCst) == generation {
                service.error(e);
            }
        }
    }
}
