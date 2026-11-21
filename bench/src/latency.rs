//! How long the user waits after letting go of the key, for long dictations.
//!
//! Joins evaluation clips into long recordings, then plays each one in real
//! time: a "whole" run recognises it after it ends (the old way), a "live"
//! run lets `Live` recognise pieces while it plays, as the controller does
//! on its 100 ms ticks. The wait is measured from the moment the recording
//! ends to the transcript being ready.
use crate::{read, samples, save, settings, target, Clip, CLOCK, ROOT};
use serde_json::{json, Value};
use speechify_core::{
    asr::Hint,
    catalog,
    context::Category,
    diff,
    live::{Cuts, Live},
    llm::{Chat, Rewriter},
    native::{Detector, Engine, DETECTOR_FILE},
    pipeline::{self, Destination, Draft, Outcome, Request, RewriteCircuit},
    store::Store,
    Result,
};
use std::{
    path::Path,
    time::{Duration, Instant},
};

fn env_ms(name: &str, default: usize) -> usize {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn cuts() -> Cuts {
    use speechify_core::live::*;
    Cuts {
        min_piece: env_ms("LIVE_MIN_PIECE_MS", MIN_PIECE_MS) * 16,
        pause: env_ms("LIVE_PAUSE_MS", PAUSE_MS) * 16,
        settled: env_ms("LIVE_SETTLED_MS", SETTLED_MS) * 16,
        look_every: env_ms("LIVE_LOOK_EVERY_MS", LOOK_EVERY_MS) * 16,
        max_piece: env_ms("LIVE_MAX_PIECE_MS", MAX_PIECE_MS) * 16,
    }
}

/// Recordings of about `seconds` each, from consecutive clips with a short
/// breath between them, and the reference text of each.
fn recordings(clips: &[Clip], seconds: f64, gap_ms: usize, limit: usize) -> Result<Vec<(String, Vec<f32>, String)>> {
    let mut out = Vec::new();
    let (mut ids, mut pcm, mut text) = (Vec::new(), Vec::new(), Vec::new());
    for clip in clips {
        pcm.extend(samples(&clip.audio)?);
        pcm.extend(std::iter::repeat_n(0.0, gap_ms * 16));
        ids.push(clip.id.clone());
        text.push(clip.reference.clone());
        if pcm.len() as f64 >= seconds * 16_000.0 {
            out.push((ids.join("+"), std::mem::take(&mut pcm), text.join(" ")));
            ids.clear();
            text.clear();
            if out.len() == limit {
                break;
            }
        }
    }
    Ok(out)
}

fn request(audio: &[f32]) -> Request<'_> {
    Request {
        audio,
        target: target(),
        command: false,
        selection: "",
        clock: CLOCK,
    }
}

fn raw_of(outcome: Outcome) -> String {
    match outcome {
        Outcome::Text { record, .. } => record.raw,
        _ => String::new(),
    }
}

/// Saved rows count as done only for the same settings: a different
/// model, device, gap or cut setting is a different experiment, and mixing
/// them in one file is refused rather than silently skipped.
fn check_resumable(rows: &[Value], setup: &Value) -> Result<()> {
    match rows.iter().find(|row| row["setup"] != *setup) {
        Some(row) => Err(format!(
            "The output already holds results for other settings ({}); use a new file",
            row["setup"]
        )),
        None => Ok(()),
    }
}

/// The output's folder, made before any time is spent measuring.
fn prepare(output: &Path) -> Result<Vec<Value>> {
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    if output.exists() {
        read(output)
    } else {
        Ok(vec![])
    }
}

pub fn run(manifest: &Path, output: &Path, model: &str, seconds: f64, limit: usize, modes: &[String]) -> Result<()> {
    if let Some(unknown) = modes
        .iter()
        .find(|mode| !["whole", "live", "pieces"].contains(&mode.as_str()))
    {
        return Err(format!("Unknown mode {unknown}: use whole, live or pieces"));
    }
    let clips: Vec<Clip> = read(manifest)?;
    let runtime = std::env::var("SPEECHIFY_NATIVE_DIR").map_err(|e| e.to_string())?;
    let runtime = Path::new(&runtime);
    let path = catalog::path(&Path::new(ROOT).join("models"), &catalog::find(model)?);
    let device = std::env::var("LIVE_DEVICE").unwrap_or_else(|_| "cpu".into());
    let detector = Detector::open(runtime, &runtime.join(DETECTOR_FILE), &device)?;
    let mut engine = Engine::open(runtime, &path, &device)?.with_detector(detector);
    let mut s = settings(model);
    s.speech.device = device.clone();
    let store = Store::memory();
    let gap = env_ms("LIVE_GAP_MS", 350);
    let setup =
        json!({"model": model, "device": device, "gap_ms": gap, "seconds": seconds, "cuts": format!("{:?}", cuts())});
    let mut rows = prepare(output)?;
    check_resumable(&rows, &setup)?;
    // One warm-up, so the first measured run does not pay for it.
    let warm = samples(&clips[0].audio)?;
    pipeline::run(&request(&warm), &s, &store, &mut engine, None)?;
    for (id, pcm, reference) in recordings(&clips, seconds, gap, limit)? {
        for mode in modes {
            if rows
                .iter()
                .any(|r| r["id"] == id.as_str() && r["mode"] == mode.as_str())
            {
                continue;
            }
            let duration = Duration::from_secs_f64(pcm.len() as f64 / 16_000.0);
            let mut live = Live::new(cuts());
            let mut busy = Duration::ZERO;
            let mut advance_errors = 0;
            let started = Instant::now();
            let ends = started + duration;
            if mode == "pieces" {
                // Not in real time: as if recognition always kept up, for tuning where pieces end.
                let vocabulary = store.vocabulary();
                let hint = Hint {
                    language: "en",
                    vocabulary: &vocabulary,
                    previous: "",
                };
                let mut available = 0;
                while available < pcm.len() {
                    available = (available + 1600).min(pcm.len());
                    let begun = Instant::now();
                    if live.advance(&pcm[live.done()..available], &mut engine, &hint).is_err() {
                        advance_errors += 1;
                    }
                    busy += begun.elapsed();
                }
            } else if mode == "live" {
                let vocabulary = store.vocabulary();
                let hint = Hint {
                    language: "en",
                    vocabulary: &vocabulary,
                    previous: "",
                };
                loop {
                    let available = ((started.elapsed().as_secs_f64() * 16_000.0) as usize).min(pcm.len());
                    if available == pcm.len() {
                        break;
                    }
                    let begun = Instant::now();
                    if live.advance(&pcm[live.done()..available], &mut engine, &hint).is_err() {
                        advance_errors += 1;
                    }
                    busy += begun.elapsed();
                    std::thread::sleep(Duration::from_millis(100));
                }
            } else if mode == "whole" {
                std::thread::sleep(duration);
            }
            let left_ms = (pcm.len() - live.done()) as f64 / 16.0;
            let finish_started = Instant::now();
            let outcome = pipeline::run_live(
                &request(&pcm),
                pipeline::Draft::new(live),
                &s,
                &store,
                &mut engine,
                None,
                (&mut RewriteCircuit::default(), 0),
            )?;
            // Offline runs have no real end time: count only the finishing work.
            let wait_ms = if mode == "pieces" {
                finish_started.elapsed().as_secs_f64() * 1000.0
            } else {
                Instant::now().saturating_duration_since(ends).as_secs_f64() * 1000.0
            };
            let raw = raw_of(outcome);
            let wer = diff::wer(&reference, &raw);
            eprintln!(
                "{model} {mode} {:.0}s: wait {wait_ms:.0} ms, WER {:.1}%, left {left_ms:.0} ms, busy {:.0}%",
                duration.as_secs_f64(),
                wer * 100.0,
                busy.as_secs_f64() * 100.0 / duration.as_secs_f64()
            );
            rows.push(json!({
                "id": id, "mode": mode, "model": model, "device": engine.device(),
                "audio_s": duration.as_secs_f64(), "wait_ms": wait_ms, "left_ms": left_ms,
                "busy_ms": busy.as_secs_f64() * 1000.0, "advance_errors": advance_errors,
                "wer": wer, "reference": reference, "raw": raw,
                "setup": setup,
            }));
            save(output, &rows)?;
        }
    }
    Ok(())
}

/// Cleanup in blocks while speaking against cleanup of the whole at the end,
/// on the same recognised words, with a real language model.
pub fn blocks(manifest: &Path, output: &Path, model: &str, seconds: f64, limit: usize) -> Result<()> {
    let clips: Vec<Clip> = read(manifest)?;
    let runtime = std::env::var("SPEECHIFY_NATIVE_DIR").map_err(|e| e.to_string())?;
    let runtime = Path::new(&runtime);
    let path = catalog::path(&Path::new(ROOT).join("models"), &catalog::find(model)?);
    let detector = Detector::open(runtime, &runtime.join(DETECTOR_FILE), "cpu")?;
    let mut engine = Engine::open(runtime, &path, "cpu")?.with_detector(detector);
    let url = std::env::var("LIVE_LLM_URL").unwrap_or_else(|_| "http://127.0.0.1:11434".into());
    let llm_model = std::env::var("LIVE_LLM_MODEL").unwrap_or_else(|_| "gemma3:1b".into());
    let chat = Chat::new(&url, "", &llm_model, Duration::from_secs(600));
    let mut s = settings(model);
    s.rewrite.enabled = true;
    s.rewrite.model = llm_model.clone();
    let store = Store::memory();
    let gap = env_ms("LIVE_GAP_MS", 350);
    let setup =
        json!({"model": model, "llm": llm_model, "gap_ms": gap, "seconds": seconds, "cuts": format!("{:?}", cuts())});
    let mut rows = prepare(output)?;
    check_resumable(&rows, &setup)?;
    let hint_vocabulary: Vec<String> = vec![];
    let hint = Hint {
        language: "en",
        vocabulary: &hint_vocabulary,
        previous: "",
    };
    for (id, pcm, reference) in recordings(&clips, seconds, gap, limit)? {
        if rows.iter().any(|r| r["id"] == id.as_str()) {
            continue;
        }
        let target = target();
        let to = Destination {
            target: &target,
            category: Category::Work,
            settings: &s,
            store: &store,
            rewriter: Some(&chat as &dyn Rewriter),
        };
        let mut draft = Draft::new(Live::new(cuts()));
        let mut available = 0;
        let mut early_ms = 0.0;
        let mut blocks = 0;
        while available < pcm.len() {
            available = (available + 1600).min(pcm.len());
            if let Ok(true) = draft
                .live
                .advance(&pcm[draft.live.done()..available], &mut engine, &hint)
            {
                let begun = Instant::now();
                // A fresh breaker each time: this measures every block, not the breaker.
                if draft.clean(&to, (&mut RewriteCircuit::default(), 0)) {
                    blocks += 1;
                    early_ms += begun.elapsed().as_secs_f64() * 1000.0;
                }
            }
        }
        let outcome = pipeline::run_live(
            &request(&pcm),
            draft,
            &s,
            &store,
            &mut engine,
            Some(&chat),
            (&mut RewriteCircuit::default(), 0),
        )?;
        let Outcome::Text { record, .. } = outcome else { continue };
        let started = Instant::now();
        let whole = pipeline::compose_guarded(
            &record.raw,
            &target,
            Category::Work,
            &s,
            &store,
            Some(&chat),
            (&mut RewriteCircuit::default(), 0),
        );
        let whole_ms = started.elapsed().as_secs_f64() * 1000.0;
        let whole = whole.unwrap_or_default();
        eprintln!(
            "{id}: blocks {blocks} early {early_ms:.0} ms, last block {} ms vs whole {whole_ms:.0} ms; WER blocks {:.1}% whole {:.1}%",
            record.rewrite_ms,
            diff::wer(&reference, &record.text) * 100.0,
            diff::wer(&reference, &whole.text) * 100.0
        );
        rows.push(json!({
            "id": id, "reference": reference, "raw": record.raw,
            "blocks": blocks, "early_ms": early_ms, "last_block_ms": record.rewrite_ms,
            "blocks_text": record.text, "blocks_rewritten": record.rewritten,
            "whole_ms": whole_ms, "whole_text": whole.text, "whole_rewritten": whole.rewritten,
            "whole_fallback": whole.fallback, "setup": setup,
        }));
        save(output, &rows)?;
    }
    Ok(())
}
