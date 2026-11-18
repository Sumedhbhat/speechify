use serde_json::{json, Value};
use speechify_core::{
    audio, catalog,
    context::{Category, Target},
    llm::Chat,
    native::{Detector, Engine},
    pipeline::{self, Outcome, Request},
    settings::{Settings, Style},
    store::{Clock, Store},
    Result,
};
use std::{
    path::Path,
    time::{Duration, Instant},
};

fn speech() -> Result<()> {
    let runtime = Path::new("src-tauri/resources/native");
    let models = Path::new("benchmark-models");
    let audio = audio::decode_wav(include_bytes!("../../core/tests/fixtures/jfk.wav"))?;
    let store = Store::memory();
    let mut settings = Settings::default();
    settings.rewrite.enabled = false;
    for id in ["whisper-large-v3-turbo", "parakeet-tdt-0.6b-v3", "parakeet-tdt-0.6b-v2"] {
        let model = catalog::find(id)?;
        let weight = if catalog::path(models, &model).is_file() {
            catalog::path(models, &model)
        } else {
            catalog::download(models, &model, &mut |_, _| true)?
        };
        let engine = Engine::open(runtime, &weight, "cpu")?;
        let device = engine.device().to_string();
        let mut engine = engine.with_detector(Detector::open(
            runtime,
            &runtime.join("silero-vad-v6.2-F32.gguf"),
            "cpu",
        )?);
        for run in 0..=5 {
            for (seconds, pcm) in [(4, &audio[..4 * 16000]), (11, audio.as_slice())] {
                let request = Request {
                    audio: pcm,
                    target: Target::default(),
                    command: false,
                    selection: "",
                    clock: Clock { now: 0, offset_min: 0 },
                };
                let Outcome::Text { record, .. } = pipeline::run(&request, &settings, &store, &mut engine, None)?
                else {
                    return Err(format!("{id} returned no text on the {seconds}s clip"));
                };
                println!(
                    "{}",
                    json!({"model": id, "device": device, "run": run,
                    "clip_seconds": seconds, "ms": record.total_ms, "words": record.words, "text": record.raw})
                );
            }
        }
    }
    Ok(())
}

fn cleanup(backend: &str, url: &str, model: &str, timeout: u64) -> Result<()> {
    let rows: Vec<Value> = serde_json::from_str(include_str!("../cases.json")).map_err(|e| e.to_string())?;
    let extra: Vec<Value> = serde_json::from_str(include_str!("../latency-cases.json")).map_err(|e| e.to_string())?;
    let mut settings = Settings::default();
    settings.styles.work = Style::Formal;
    let target = Target {
        app: "Slack".into(),
        ..Target::default()
    };
    let store = Store::memory();
    let chat = Chat::new(url, "", model, Duration::from_secs(timeout));
    for row in rows.into_iter().chain(extra) {
        let raw = row["raw"].as_str().ok_or("missing raw text")?;
        if raw.split_whitespace().count() < settings.rewrite.min_words {
            continue;
        }
        let client = (backend != "rules").then_some(&chat as &dyn speechify_core::llm::Rewriter);
        let start = Instant::now();
        let result = pipeline::compose(raw, &target, Category::Work, &settings, &store, client);
        let text = result.as_ref().map_or("", |r| r.text.as_str());
        println!(
            "{}",
            json!({"backend":backend, "model":model, "id":row["id"],
            "exact":row["expected"].as_str() == Some(text), "text": text, "expected":row["expected"],
            "ms": result.as_ref().map_or(start.elapsed().as_millis() as u64, |r| r.rewrite_ms.into()),
            "fallback": result.and_then(|r| r.fallback)})
        );
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.as_slice() {
        [mode] if mode == "speech" => speech(),
        [backend, url, model, timeout] => cleanup(backend, url, model, timeout.parse().expect("timeout seconds")),
        _ => Err("expected speech or BACKEND URL MODEL TIMEOUT_SECONDS".into()),
    };
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
