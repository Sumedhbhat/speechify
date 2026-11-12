//! Quiet-speech study: the loudness trim against the voice detector, on
//! labelled recordings of speech and of non-speech sounds.

use serde::Deserialize;
use serde_json::{json, Value};
use speechify_core::{
    asr::{isolate, Hint, Recognizer},
    audio, catalog,
    diff::wer,
    native::{Detection, Detector, Engine, DETECTOR_FILE},
    Result,
};
use std::{collections::HashSet, fs, io::Write, path::Path, time::Instant};

/// Detector thresholds compared; the app's own setting is among them.
const THRESHOLDS: &[f64] = &[0.15, 0.25, 0.35, 0.5, 0.65];

/// The voice detector alone, so the app's own `asr::isolate` can be used
/// to decide what is kept; it never transcribes.
struct DetectOnly(Detector);

impl Recognizer for DetectOnly {
    fn transcribe(&mut self, _: &[f32], _: &Hint) -> Result<String> {
        Err("detection only".into())
    }
    fn name(&self) -> &str {
        "detector"
    }
    fn detect(&mut self, audio: &[f32], threshold: f64) -> Option<Result<Vec<std::ops::Range<usize>>>> {
        Some(self.0.speech(
            audio,
            &Detection {
                threshold,
                ..audio::DETECTION
            },
        ))
    }
}

#[derive(Deserialize)]
struct Clip {
    id: String,
    path: String,
    label: String,
    kind: String,
    #[serde(default)]
    level_dbfs: Option<f64>,
    #[serde(default)]
    speech_level_dbfs: Option<f64>,
    #[serde(default)]
    reference: String,
    /// Variants of one recording (levels, added noise) share a family and
    /// always land on the same side of the development/report split.
    #[serde(default)]
    family: Option<String>,
    /// Nominal level: a number in dB, or "as-recorded".
    #[serde(default)]
    level: Value,
    #[serde(default)]
    whisper_quality: Option<String>,
}

fn ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1000.0
}

/// Stable development/report split, independent of clip order.
fn split(id: &str) -> &'static str {
    let hash = id.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100_0000_01b3)
    });
    if hash % 2 == 0 {
        "dev"
    } else {
        "report"
    }
}

pub fn run(manifest: &Path, output: &Path, model: &str, asr: bool) -> Result<()> {
    let runtime = std::env::var("SPEECHIFY_NATIVE_DIR").map_err(|e| e.to_string())?;
    let runtime = Path::new(&runtime);
    let clips: Vec<Clip> =
        serde_json::from_slice(&fs::read(manifest).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    fs::create_dir_all(output).map_err(|e| e.to_string())?;
    let results = output.join("results.jsonl");
    let done: HashSet<String> = fs::read_to_string(&results)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            serde_json::from_str::<Value>(line).ok()?["id"]
                .as_str()
                .map(str::to_string)
        })
        .collect();
    let mut detector = DetectOnly(Detector::open(runtime, &runtime.join(DETECTOR_FILE), "cpu")?);
    let mut engine = if asr {
        let path = catalog::path(&Path::new(crate::ROOT).join("models"), &catalog::find(model)?);
        Some(Engine::open(runtime, &path, "cpu")?)
    } else {
        None
    };
    let mut out = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&results)
        .map_err(|e| e.to_string())?;
    let hint = Hint {
        language: "en",
        vocabulary: &[],
    };
    for (n, clip) in clips.iter().enumerate() {
        if done.contains(&clip.id) {
            continue;
        }
        // Clip paths may be relative to the manifest's own folder.
        let path = manifest.parent().unwrap_or(Path::new(".")).join(&clip.path);
        let raw = audio::decode_wav(&fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?)?;
        let started = Instant::now();
        let old = audio::trim(&raw);
        let old_ms = ms(started);
        // The app's own decision, exactly as dictation makes it.
        let started = Instant::now();
        let chosen = isolate(&mut detector, &raw)?;
        let detect_ms = ms(started);
        let normalized = audio::normalize(&raw);
        let span = audio::loudness_span(&raw);
        let mut sweep = serde_json::Map::new();
        for threshold in THRESHOLDS {
            let how = Detection {
                threshold: *threshold,
                ..audio::DETECTION
            };
            let regions = match &normalized {
                Some(loud) => detector.0.speech(loud, &how)?,
                None => vec![],
            };
            let trimmed = normalized.as_ref().map_or_else(audio::Trimmed::default, |loud| {
                audio::trim_to(loud, &regions, span.clone())
            });
            sweep.insert(
                format!("{threshold}"),
                json!({"kept": !trimmed.samples.is_empty(), "speech_ms": trimmed.speech_ms}),
            );
        }
        let mut row = json!({
            "id": clip.id, "label": clip.label, "kind": clip.kind, "split": split(clip.family.as_deref().unwrap_or(&clip.id)),
            "family": clip.family, "level": clip.level, "whisper_quality": clip.whisper_quality,
            "level_dbfs": clip.level_dbfs, "speech_level_dbfs": clip.speech_level_dbfs,
            "duration_ms": raw.len() as u64 * 1000 / audio::RATE as u64,
            "no_signal": normalized.is_none(),
            "old": {"kept": !old.samples.is_empty(), "speech_ms": old.speech_ms, "ms": old_ms},
            "new": {"kept": !chosen.samples.is_empty(), "speech_ms": chosen.speech_ms, "ms": detect_ms},
            "sweep": sweep,
        });
        if let (Some(engine), "speech", false) = (engine.as_mut(), clip.label.as_str(), clip.reference.is_empty()) {
            for (key, trimmed) in [("old", &old), ("new", &chosen)] {
                // A dropped recording types nothing: every reference word is missed.
                let (text, took) = if trimmed.samples.is_empty() {
                    (String::new(), 0.0)
                } else {
                    let started = Instant::now();
                    (engine.transcribe(&trimmed.samples, &hint)?, ms(started))
                };
                row[key]["text"] = json!(text);
                row[key]["wer"] = json!(wer(&clip.reference, &text));
                row[key]["asr_ms"] = json!(took);
            }
            row["reference"] = json!(clip.reference);
        }
        writeln!(out, "{row}").map_err(|e| e.to_string())?;
        if n % 25 == 0 {
            eprintln!("{n}/{} {}", clips.len(), clip.id);
        }
    }
    Ok(())
}

/// For a few named clips: the loudness envelope, where each method cuts,
/// and what the recogniser heard from each cut. Feeds the waveform figure.
pub fn examples(manifest: &Path, output: &Path, model: &str, ids: &[String]) -> Result<()> {
    let runtime = std::env::var("SPEECHIFY_NATIVE_DIR").map_err(|e| e.to_string())?;
    let runtime = Path::new(&runtime);
    let clips: Vec<Clip> =
        serde_json::from_slice(&fs::read(manifest).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let mut detector = DetectOnly(Detector::open(runtime, &runtime.join(DETECTOR_FILE), "cpu")?);
    let path = catalog::path(&Path::new(crate::ROOT).join("models"), &catalog::find(model)?);
    let mut engine = Engine::open(runtime, &path, "cpu")?;
    let hint = Hint {
        language: "en",
        vocabulary: &[],
    };
    let mut rows = Vec::new();
    for id in ids {
        let clip = clips
            .iter()
            .find(|clip| &clip.id == id)
            .ok_or(format!("No clip {id}"))?;
        let raw = audio::decode_wav(
            &fs::read(manifest.parent().unwrap_or(Path::new(".")).join(&clip.path)).map_err(|e| e.to_string())?,
        )?;
        // 20 ms frames, as dBFS, floored at -90.
        let envelope: Vec<f64> = raw
            .chunks(320)
            .map(|frame| {
                let power = frame.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / frame.len() as f64;
                (10.0 * power.max(1e-9).log10()).max(-90.0)
            })
            .collect();
        let mut cut = |trimmed: &audio::Trimmed| -> Result<Value> {
            let text =
                if trimmed.samples.is_empty() { String::new() } else { engine.transcribe(&trimmed.samples, &hint)? };
            Ok(
                json!({"kept": !trimmed.samples.is_empty(), "start_s": trimmed.span.start as f64 / 16_000.0, "end_s": trimmed.span.end as f64 / 16_000.0, "text": text}),
            )
        };
        let old = cut(&audio::trim(&raw))?;
        let new = cut(&isolate(&mut detector, &raw)?)?;
        rows.push(
            json!({"id": id, "label": clip.label, "kind": clip.kind, "level": clip.level, "reference": clip.reference,
            "duration_s": raw.len() as f64 / 16_000.0, "envelope_db": envelope, "old": old, "new": new}),
        );
    }
    fs::write(output, serde_json::to_string_pretty(&rows).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}
