//! Headless, incremental evaluation of the production dictation and teacher paths.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use speechify_core::{
    asr::{Hint, Recognizer},
    audio, catalog,
    context::{Category, Target},
    llm::{Chat, Rewriter},
    native::Engine,
    pipeline::{self, Outcome, Request},
    settings::{Settings, Style},
    store::{Clock, Store},
    teacher, Result,
};
mod quiet;

use std::{
    cell::RefCell,
    fs,
    path::Path,
    time::{Duration, Instant},
};

pub(crate) const ROOT: &str = "/workspaces/.cache-speechify/eval";
const CLOCK: Clock = Clock {
    now: 1_794_355_200,
    offset_min: 330,
};

#[derive(Clone, Deserialize)]
struct Clip {
    id: String,
    audio: String,
    reference: String,
    scenario: String,
    split: String,
    duration_s: f64,
}

#[derive(Default, Serialize, Deserialize)]
struct Results {
    model: String,
    load_ms: f64,
    #[serde(default)]
    load_observations_ms: Vec<f64>,
    device: String,
    #[serde(default)]
    vocabulary_supported: bool,
    rows: Vec<Value>,
}

fn read<T: for<'de> Deserialize<'de>>(p: &Path) -> Result<T> {
    serde_json::from_slice(&fs::read(p).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}
fn save<T: Serialize>(p: &Path, value: &T) -> Result<()> {
    let tmp = p.with_extension("json.part");
    fs::write(&tmp, serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    fs::rename(tmp, p).map_err(|e| e.to_string())
}
fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}
fn samples(path: &str) -> Result<Vec<f32>> {
    audio::decode_wav(&fs::read(path).map_err(|e| e.to_string())?)
}
fn target() -> Target {
    Target {
        app: "Slack".into(),
        ..Target::default()
    }
}
fn settings(model: &str) -> Settings {
    let mut s = Settings::default();
    s.speech.model = model.into();
    s.speech.device = "cpu".into();
    s.rewrite.enabled = false;
    s.styles.work = Style::Formal;
    s.teacher.speech_model = "whisper-large-v3-turbo".into();
    s.teacher.batch = 1;
    s
}

struct Timed {
    engine: Engine,
    elapsed_ms: f64,
    raw: String,
}
impl Recognizer for Timed {
    fn transcribe(&mut self, audio: &[f32], hint: &Hint) -> Result<String> {
        let start = Instant::now();
        let result = self.engine.transcribe(audio, hint);
        self.elapsed_ms = ms(start);
        self.raw = result.as_ref().cloned().unwrap_or_default();
        result
    }
    fn name(&self) -> &str {
        self.engine.name()
    }
}
fn open(model: &str) -> Result<(Timed, f64)> {
    let runtime = std::env::var("SPEECHIFY_NATIVE_DIR").map_err(|e| e.to_string())?;
    let path = catalog::path(&Path::new(ROOT).join("models"), &catalog::find(model)?);
    let start = Instant::now();
    let engine = Engine::open(Path::new(&runtime), &path, "cpu")?;
    let load_ms = ms(start);
    Ok((
        Timed {
            engine,
            elapsed_ms: 0.0,
            raw: String::new(),
        },
        load_ms,
    ))
}
fn cpu_ticks() -> u64 {
    let stat = fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let fields: Vec<_> = stat
        .rsplit_once(')')
        .map_or("", |(_, s)| s)
        .split_whitespace()
        .collect();
    [11, 12]
        .iter()
        .filter_map(|i| fields.get(*i)?.parse::<u64>().ok())
        .sum()
}
fn pipeline_row(c: &Clip, s: &Settings, store: &Store, engine: &mut Timed) -> Result<Value> {
    let pcm = samples(&c.audio)?;
    let trim = Instant::now();
    let trimmed = audio::trim(&pcm);
    let trim_ms = ms(trim);
    engine.raw.clear();
    engine.elapsed_ms = 0.0;
    let start = Instant::now();
    let ticks = cpu_ticks();
    let outcome = pipeline::run(
        &Request {
            audio: &pcm,
            target: target(),
            command: false,
            selection: "",
            clock: CLOCK,
        },
        s,
        store,
        engine,
        None,
    )?;
    let elapsed = ms(start);
    let cpu_ms = (cpu_ticks() - ticks) as f64 * 10.0;
    let (raw, text, record, status) = match outcome {
        Outcome::Text { record, .. } => (record.raw.clone(), record.text.clone(), Some(record), "text"),
        Outcome::Silent => (engine.raw.clone(), String::new(), None, "silent"),
        Outcome::Nothing => (engine.raw.clone(), String::new(), None, "nothing"),
    };
    let rules_start = Instant::now();
    let composed = pipeline::compose(&raw, &target(), Category::Work, s, store, None);
    let rules_ms = ms(rules_start);
    Ok(
        json!({"id": c.id, "reference": c.reference, "scenario": c.scenario, "split": c.split,
        "duration_s": c.duration_s, "trimmed_s": trimmed.samples.len() as f64 / 16000.0,
        "trim_ms": trim_ms, "recognize_ms": engine.elapsed_ms, "rules_ms": rules_ms,
        "pipeline_ms": elapsed, "cpu_ms": cpu_ms, "raw": raw, "text": text,
        "compose_replay": composed.map(|v| v.text), "record": record, "status": status}),
    )
}
fn store_path(study: &Path, scenario: &str) -> std::path::PathBuf {
    study.join(format!("{scenario}.sqlite"))
}

fn student(clips: &[Clip], study: &Path, model: &str, phase: &str) -> Result<()> {
    fs::create_dir_all(study).map_err(|e| e.to_string())?;
    let output = study.join(format!("{phase}.json"));
    let mut result: Results = if output.exists() { read(&output)? } else { Results::default() };
    if !result.model.is_empty() && result.model != model {
        return Err("Result model mismatch".into());
    }
    // A baseline cannot be resumed against a store already populated by the teacher.
    for scenario in ["general", "recurring"] {
        let store = Store::open(&store_path(study, scenario))?;
        if phase == "baseline" && store.all()?.iter().any(|d| d.reviewed_at.is_some()) {
            return Err("Baseline store already reviewed; use a new study directory".into());
        }
        if phase == "learned" && !store.pending_review(1)?.is_empty() {
            return Err("Finish teacher review first".into());
        }
    }
    let (mut engine, load_ms) = open(model)?;
    result.model = model.into();
    if result.load_observations_ms.is_empty() && result.load_ms > 0.0 {
        result.load_observations_ms.push(result.load_ms);
    }
    result.load_observations_ms.push(load_ms);
    result.load_ms = load_ms;
    result.device = engine.engine.device().into();
    result.vocabulary_supported = engine.engine.supports_vocabulary();
    for c in clips {
        if result.rows.iter().any(|v| v["id"] == c.id) {
            continue;
        }
        let learned = Store::open(&store_path(study, &c.scenario))?;
        let empty = Store::memory();
        let store = if phase == "learned" || c.split == "history" { &learned } else { &empty };
        let row = pipeline_row(c, &settings(model), store, &mut engine)?;
        if phase == "baseline" && c.split == "history" {
            if let Some(value) = row.get("record").filter(|v| !v.is_null()) {
                let mut d: speechify_core::store::Dictation =
                    serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
                d.audio = c.audio.clone();
                d.outcome = speechify_core::store::Outcome::Pasted;
                // Audio is already a stored PCM16 WAV; save precisely the same bytes as the app.
                if !store.all()?.iter().any(|old| old.audio == d.audio) {
                    store.add_dictation(&d)?;
                }
            }
        }
        eprintln!("{phase} {} {} {:.1}ms", c.scenario, c.id, engine.elapsed_ms);
        result.rows.push(row);
        save(&output, &result)?;
    }
    fs::write(
        study.join(format!("{phase}-process.txt")),
        fs::read_to_string("/proc/self/status").unwrap_or_default(),
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
fn review(clips: &[Clip], study: &Path, model: &str, history_only: bool) -> Result<()> {
    let output = study.join("teacher.json");
    let mut result: Results = if output.exists() { read(&output)? } else { Results::default() };
    let (mut engine, load_ms) = open(model)?;
    result.model = model.into();
    if result.load_observations_ms.is_empty() && result.load_ms > 0.0 {
        result.load_observations_ms.push(result.load_ms);
    }
    result.load_observations_ms.push(load_ms);
    result.load_ms = load_ms;
    result.device = engine.engine.device().into();
    result.vocabulary_supported = engine.engine.supports_vocabulary();
    // Closing the only connection checkpoints WAL. A per-clip database backup
    // lets a killed review restart without a half-applied learning operation.
    let marker = study.join("review-inflight.json");
    if marker.exists() {
        let flight: Value = read(&marker)?;
        let scenario = flight["scenario"].as_str().ok_or("Invalid checkpoint scenario")?;
        let id = flight["id"].as_str().ok_or("Invalid checkpoint clip")?;
        let db = store_path(study, scenario);
        for suffix in ["-wal", "-shm"] {
            let path = format!("{}{suffix}", db.display());
            if Path::new(&path).exists() {
                fs::remove_file(path).map_err(|e| e.to_string())?;
            }
        }
        fs::copy(study.join("review-checkpoint.sqlite"), db).map_err(|e| e.to_string())?;
        result.rows.retain(|r| r["id"] != id);
        save(&output, &result)?;
        fs::remove_file(&marker).map_err(|e| e.to_string())?;
    }
    for scenario in ["general", "recurring"] {
        loop {
            let store = Store::open(&store_path(study, scenario))?;
            let pending = store.pending_review(1)?;
            let Some(d) = pending.first() else { break };
            let c = clips
                .iter()
                .find(|c| c.audio == d.audio)
                .ok_or("Missing history clip")?;
            drop(store);
            fs::copy(store_path(study, scenario), study.join("review-checkpoint.sqlite")).map_err(|e| e.to_string())?;
            save(&marker, &json!({"scenario": scenario, "id": c.id}))?;
            let store = Store::open(&store_path(study, scenario))?;
            let start = Instant::now();
            let report = teacher::review(&store, &settings(model), &mut engine, None, &samples, CLOCK, &|| true)?;
            let row = store.dictation(d.id)?;
            result
                .rows
                .push(json!({"id": c.id, "reference": c.reference, "scenario": scenario,
                "split": "history", "duration_s": c.duration_s, "raw": row.teacher_raw,
                "text": row.teacher_text, "student_raw": row.raw, "display_wer": row.teacher_wer,
                "review_ms": ms(start), "recognize_ms": engine.elapsed_ms, "report": report}));
            save(&output, &result)?;
            save(
                &study.join(format!("{scenario}-learning.json")),
                &json!({"dictionary":store.dictionary()?, "suggestions":store.suggestions()?, "examples":store.examples(Category::Work,200), "history":store.all()?}),
            )?;
            drop(store);
            fs::remove_file(&marker).map_err(|e| e.to_string())?;
            eprintln!("teacher {scenario} {} {:.1}ms", c.id, engine.elapsed_ms);
        }
    }
    for c in clips.iter().filter(|c| c.split == "test" && !history_only) {
        if result.rows.iter().any(|v| v["id"] == c.id) {
            continue;
        }
        let row = pipeline_row(c, &settings(model), &Store::memory(), &mut engine)?;
        result.rows.push(row);
        save(&output, &result)?;
        eprintln!("teacher test {} {:.1}ms", c.id, engine.elapsed_ms);
    }
    fs::write(
        study.join("teacher-process.txt"),
        fs::read_to_string("/proc/self/status").unwrap_or_default(),
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(Deserialize)]
struct Case {
    id: String,
    source: String,
    raw: String,
    expected: String,
}
struct Capture<'a> {
    inner: &'a Chat,
    answers: RefCell<Vec<Value>>,
}
impl Rewriter for Capture<'_> {
    fn rewrite(&self, system: &str, user: &str) -> Result<String> {
        let start = Instant::now();
        let result = self.inner.rewrite(system, user);
        self.answers
            .borrow_mut()
            .push(json!({"system": system, "user": user, "answer": result, "ms":ms(start)}));
        result
    }
    fn model(&self) -> &str {
        self.inner.model()
    }
}
fn formatting(cases_path: &Path, output: &Path, llm: bool, timeout_secs: u64) -> Result<()> {
    let cases: Vec<Case> = read(cases_path)?;
    let mut rows: Vec<Value> = if output.exists() { read(output)? } else { vec![] };
    let store = Store::memory();
    let chat = Chat::new(
        "http://127.0.0.1:11434",
        "",
        "gemma3:1b",
        Duration::from_secs(timeout_secs),
    );
    let capture = Capture {
        inner: &chat,
        answers: RefCell::new(vec![]),
    };
    let mut circuit = pipeline::RewriteCircuit::default();
    let began = Instant::now();
    for c in cases {
        if rows.iter().any(|v| v["id"] == c.id) {
            continue;
        }
        let mut s = settings("");
        s.rewrite.enabled = llm;
        s.rewrite.min_words = 0;
        let start = Instant::now();
        let out = pipeline::compose_guarded(
            &c.raw,
            &target(),
            Category::Work,
            &s,
            &store,
            if llm { Some(&capture as &dyn Rewriter) } else { None },
            (&mut circuit, began.elapsed().as_millis() as u64),
        );
        let elapsed = ms(start);
        let text = out.as_ref().map(|v| v.text.as_str()).unwrap_or("");
        rows.push(json!({"id":c.id,"source":c.source,"raw":c.raw,"expected":c.expected,
            "text":text,"correct":text==c.expected,"elapsed_ms":elapsed,
            "rewritten":out.as_ref().is_some_and(|v|v.rewritten),
            "fallback":out.and_then(|v|v.fallback),"calls":capture.answers.take()}));
        save(output, &rows)?;
        eprintln!("format {} {:.1}ms", c.id, elapsed);
    }
    Ok(())
}
// A cleanup-only ablation reuses native transcripts measured earlier. It never
// loads an ASR model alongside Ollama, and never substitutes reference text.
struct Replay {
    raw: String,
}
impl Recognizer for Replay {
    fn transcribe(&mut self, _audio: &[f32], _hint: &Hint) -> Result<String> {
        Ok(self.raw.clone())
    }
    fn name(&self) -> &str {
        "measured-native-transcript-replay"
    }
}
fn cleanup_row(c: &Clip, raw: &str, phase: &str, store: &Store, chat: &Capture<'_>) -> Value {
    let mut s = settings("");
    s.rewrite.enabled = true; // Keep the everyday six-word minimum.
    let start = Instant::now();
    let composed = pipeline::compose(raw, &target(), Category::Work, &s, store, Some(chat));
    json!({"id":c.id,"scenario":c.scenario,"split":c.split,"reference":c.reference,
        "raw":raw,"text":composed.as_ref().map(|v|v.text.clone()).unwrap_or_default(),
        "rewritten":composed.as_ref().is_some_and(|v|v.rewritten),
        "fallback":composed.and_then(|v|v.fallback),"phase":phase,
        "elapsed_ms":ms(start),"calls":chat.answers.take()})
}
fn cleanup_study(clips: &[Clip], native_study: &Path, output_dir: &Path) -> Result<()> {
    fs::create_dir_all(output_dir).map_err(|e| e.to_string())?;
    let baseline: Results = read(&native_study.join("baseline.json"))?;
    let measured_teacher: Results = read(&native_study.join("teacher.json"))?;
    let output = output_dir.join("cleanup.json");
    let mut rows: Vec<Value> = if output.exists() { read(&output)? } else { vec![] };
    let chat = Chat::new("http://127.0.0.1:11434", "", "gemma3:1b", Duration::from_secs(6));
    let capture = Capture {
        inner: &chat,
        answers: RefCell::new(vec![]),
    };
    let teacher_chat = Chat::new("http://127.0.0.1:11434", "", "gemma3:1b", Duration::from_secs(180));
    let teacher_capture = Capture {
        inner: &teacher_chat,
        answers: RefCell::new(vec![]),
    };
    let lookup = |id: &str| {
        baseline
            .rows
            .iter()
            .find(|r| r["id"] == id)
            .ok_or_else(|| format!("Missing baseline {id}"))
    };
    // Preserve the actual history typed before Ollama was available. Reformatting
    // it now would change the teacher's input and repeat already measured work.
    for c in clips.iter().filter(|c| c.split == "history") {
        if rows.iter().any(|r| r["id"] == c.id && r["phase"] == "history-seed") {
            continue;
        }
        let native = lookup(&c.id)?;
        let store = Store::open(&store_path(output_dir, &c.scenario))?;
        if !native["record"].is_null() {
            let mut record: speechify_core::store::Dictation =
                serde_json::from_value(native["record"].clone()).map_err(|e| e.to_string())?;
            record.audio = c.audio.clone();
            record.outcome = speechify_core::store::Outcome::Pasted;
            if !store.all()?.iter().any(|r| r.audio == c.audio) {
                store.add_dictation(&record)?;
            }
        }
        rows.push(
            json!({"id":c.id,"scenario":c.scenario,"split":c.split,"reference":c.reference,
            "raw":native["raw"],"text":native["text"],"phase":"history-seed","calls":[]}),
        );
        save(&output, &rows)?;
    }
    let marker = output_dir.join("review-inflight.json");
    if marker.exists() {
        let flight: Value = read(&marker)?;
        let scenario = flight["scenario"].as_str().ok_or("Invalid checkpoint")?;
        let db = store_path(output_dir, scenario);
        for suffix in ["-wal", "-shm"] {
            let path = format!("{}{suffix}", db.display());
            if Path::new(&path).exists() {
                fs::remove_file(path).map_err(|e| e.to_string())?;
            }
        }
        fs::copy(output_dir.join("review-checkpoint.sqlite"), db).map_err(|e| e.to_string())?;
        rows.retain(|r| r["phase"] != "review" || r["id"] != flight["id"]);
        save(&output, &rows)?;
        fs::remove_file(&marker).map_err(|e| e.to_string())?;
    }
    for scenario in ["general", "recurring"] {
        loop {
            let store = Store::open(&store_path(output_dir, scenario))?;
            let pending = store.pending_review(1)?;
            let Some(d) = pending.first() else { break };
            let c = clips.iter().find(|c| c.audio == d.audio).ok_or("Missing clip")?;
            let native = measured_teacher
                .rows
                .iter()
                .find(|r| r["id"] == c.id)
                .ok_or("Missing measured teacher")?;
            let mut replay = Replay {
                raw: native["raw"].as_str().ok_or("Missing teacher raw")?.into(),
            };
            drop(store);
            fs::copy(
                store_path(output_dir, scenario),
                output_dir.join("review-checkpoint.sqlite"),
            )
            .map_err(|e| e.to_string())?;
            save(&marker, &json!({"scenario":scenario,"id":c.id}))?;
            let store = Store::open(&store_path(output_dir, scenario))?;
            let mut s = settings("");
            s.rewrite.enabled = true;
            let start = Instant::now();
            let report = teacher::review(
                &store,
                &s,
                &mut replay,
                Some(&teacher_capture),
                &samples,
                CLOCK,
                &|| true,
            )?;
            let reviewed = store.dictation(d.id)?;
            rows.push(json!({"id":c.id,"scenario":scenario,"phase":"review","raw":reviewed.teacher_raw,
                "text":reviewed.teacher_text,"reference":c.reference,"report":report,"calls":teacher_capture.answers.take(),"elapsed_ms":ms(start)}));
            save(&output, &rows)?;
            save(
                &output_dir.join(format!("{scenario}-learning.json")),
                &json!({"dictionary":store.dictionary()?,"suggestions":store.suggestions()?,"examples":store.examples(Category::Work,200)}),
            )?;
            drop(store);
            fs::remove_file(&marker).map_err(|e| e.to_string())?;
            eprintln!("cleanup teacher {scenario} {}", c.id);
        }
    }
    for c in clips.iter().filter(|c| c.split == "test") {
        for phase in ["test-empty", "test-learned"] {
            if rows.iter().any(|r| r["id"] == c.id && r["phase"] == phase) {
                continue;
            }
            let store = if phase == "test-empty" {
                Store::memory()
            } else {
                Store::open(&store_path(output_dir, &c.scenario))?
            };
            let raw = lookup(&c.id)?["raw"].as_str().ok_or("Missing raw")?;
            rows.push(cleanup_row(c, raw, phase, &store, &capture));
            save(&output, &rows)?;
            eprintln!("cleanup {phase} {}", c.id);
        }
    }
    Ok(())
}

fn quiet_probe(model: &str, path: &str, output: &Path) -> Result<()> {
    let pcm = samples(path)?;
    let gained: Vec<f32> = pcm.iter().map(|v| (v * 4.0).clamp(-1.0, 1.0)).collect();
    let (mut engine, load_ms) = open(model)?;
    let direct = engine.transcribe(
        &pcm,
        &Hint {
            language: "en",
            vocabulary: &[],
        },
    )?;
    let mut results = vec![];
    for (label, data) in [("original", pcm.as_slice()), ("gain4", gained.as_slice())] {
        let trimmed = audio::trim(data);
        let outcome = pipeline::run(
            &Request {
                audio: data,
                target: target(),
                command: false,
                selection: "",
                clock: CLOCK,
            },
            &settings(model),
            &Store::memory(),
            &mut engine,
            None,
        )?;
        let (status, raw, text) = match outcome {
            Outcome::Text { record, .. } => ("text", record.raw, record.text),
            Outcome::Silent => ("silent", String::new(), String::new()),
            Outcome::Nothing => ("nothing", String::new(), String::new()),
        };
        results
            .push(json!({"input":label,"trimmed_samples":trimmed.samples.len(),"status":status,"raw":raw,"text":text}));
    }
    save(
        output,
        &json!({"model":model,"audio":path,"load_ms":load_ms,"direct_without_trim":direct,"pipeline":results}),
    )
}

fn example_probe(study: &Path, output: &Path) -> Result<()> {
    let baseline: Results = read(&study.join("baseline.json"))?;
    let measured: Results = read(&study.join("teacher.json"))?;
    let student = baseline
        .rows
        .iter()
        .find(|r| r["id"] == "490")
        .ok_or("Missing clip 490")?;
    let teacher = measured
        .rows
        .iter()
        .find(|r| r["id"] == "490")
        .ok_or("Missing teacher clip 490")?;
    let mut d: speechify_core::store::Dictation =
        serde_json::from_value(student["record"].clone()).map_err(|e| e.to_string())?;
    d.audio = "490.wav".into();
    d.outcome = speechify_core::store::Outcome::Pasted;
    let store = Store::memory();
    store.add_dictation(&d)?;
    let mut recognizer = Replay {
        raw: teacher["raw"].as_str().ok_or("Missing teacher raw")?.into(),
    };
    let report = teacher::review(
        &store,
        &settings(""),
        &mut recognizer,
        None,
        &|name| samples(&Path::new(ROOT).join("audio").join(name).to_string_lossy()),
        CLOCK,
        &|| true,
    )?;
    save(
        output,
        &json!({"id":"490","reference":student["reference"],"student_raw":d.raw,
        "teacher_raw":teacher["raw"],"report":report,"dictionary":store.dictionary()?,
        "examples":store.examples(Category::Work,3)}),
    )
}

fn cleanup_probe(clips: &[Clip], native: &Path, learned: &Path, output: &Path) -> Result<()> {
    let baseline: Results = read(&native.join("baseline.json"))?;
    let measured: Vec<Value> = read(&learned.join("cleanup.json"))?;
    let chat = Chat::new("http://127.0.0.1:11434", "", "gemma3:1b", Duration::from_secs(6));
    let capture = Capture {
        inner: &chat,
        answers: RefCell::new(vec![]),
    };
    let mut rows: Vec<Value> = if output.exists() { read(output)? } else { vec![] };
    for c in clips.iter().filter(|c| c.split == "test") {
        if !measured.iter().any(|r| r["id"] == c.id && r["rewritten"] == true) {
            continue;
        }
        let raw = baseline
            .rows
            .iter()
            .find(|r| r["id"] == c.id)
            .and_then(|r| r["raw"].as_str())
            .ok_or("Missing probe raw")?;
        for (cycle, phases) in [["learned", "empty"], ["empty", "learned"]].iter().enumerate() {
            for phase in phases {
                let label = format!("probe-{cycle}-{phase}");
                if rows.iter().any(|r| r["id"] == c.id && r["phase"] == label) {
                    continue;
                }
                let store =
                    if *phase == "empty" { Store::memory() } else { Store::open(&store_path(learned, &c.scenario))? };
                rows.push(cleanup_row(c, raw, &label, &store, &capture));
                save(output, &rows)?;
            }
        }
    }
    Ok(())
}

// Development uses history only. Both mechanisms replay exactly the same
// saved student/teacher transcripts; reference words are used only by analysis.
fn history_audit(clips: &[Clip], native: &Path, output: &Path) -> Result<()> {
    fs::create_dir_all(output).map_err(|e| e.to_string())?;
    let baseline: Results = read(&native.join("baseline.json"))?;
    let measured: Results = read(&native.join("teacher.json"))?;
    let mut rows = Vec::new();
    for scenario in ["general", "recurring"] {
        let store = Store::open(&store_path(output, scenario))?;
        if !store.all()?.is_empty() {
            return Err("History audit needs fresh stores".into());
        }
        let naive = Store::memory();
        for c in clips.iter().filter(|c| c.split == "history" && c.scenario == scenario) {
            let Some(student) = baseline.rows.iter().find(|r| r["id"] == c.id) else { continue };
            let Some(teacher) = measured.rows.iter().find(|r| r["id"] == c.id) else { continue };
            let d: speechify_core::store::Dictation =
                serde_json::from_value(student["record"].clone()).map_err(|e| e.to_string())?;
            let raw = &d.raw;
            let heard = teacher["raw"].as_str().ok_or("Missing history teacher")?;
            for correction in speechify_core::learn::corrections(raw, heard) {
                // Candidate ablation: remove independent spelling evidence.
                if speechify_core::learn::corroborated(&correction, raw, std::slice::from_ref(&correction.to)) {
                    naive.add_word(&correction.from, &correction.to, "teacher", CLOCK)?;
                }
            }
            store.add_dictation(&speechify_core::store::Dictation {
                audio: c.audio.clone(),
                ..d
            })?;
            teacher::review(
                &store,
                &settings(""),
                &mut Replay { raw: heard.into() },
                None,
                &samples,
                CLOCK,
                &|| true,
            )?;
        }
        save(
            &output.join(format!("{scenario}-learning.json")),
            &json!({"dictionary":store.dictionary()?, "suggestions":store.suggestions()?, "examples":store.examples(Category::Work,200), "history":store.all()?, "naive_dictionary":naive.dictionary()?}),
        )?;
        for c in clips.iter().filter(|c| c.split == "history" && c.scenario == scenario) {
            let Some(student) = baseline.rows.iter().find(|r| r["id"] == c.id) else { continue };
            let raw = student["raw"].as_str().ok_or("Missing history student")?;
            for (phase, dictionary) in [
                ("empty", &Store::memory()),
                ("corroborated", &store),
                ("single-sighting", &naive),
            ] {
                let composed = pipeline::compose(raw, &target(), Category::Work, &settings(""), dictionary, None);
                rows.push(json!({"id":c.id,"scenario":scenario,"split":"history","reference":c.reference,"raw":raw,"phase":phase,"text":composed.map(|v|v.text).unwrap_or_default()}));
            }
        }
    }
    save(&output.join("history.json"), &rows)
}

fn main() {
    if let Err(e) = run() {
        eprintln!("ERROR: {e}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("download") => {
            let model = catalog::find(args.get(2).ok_or("model required")?)?;
            let mut previous = 0;
            let path = catalog::download(&Path::new(ROOT).join("models"), &model, &mut |done,total| {
                if done / 50_000_000 != previous { previous = done / 50_000_000; eprintln!("{done}/{total}"); } true
            })?;
            save(&Path::new(ROOT).join(format!("{}-pin.json",model.id)), &model)?;
            println!("{}",path.display()); Ok(())
        }
        Some("student" | "teacher") => {
            let manifest = args.get(2).ok_or("manifest required")?;
            let study = Path::new(args.get(3).ok_or("study directory required")?);
            let model = args.get(4).ok_or("model required")?;
            let clips: Vec<Clip> = read(Path::new(manifest))?;
            if args[1] == "teacher" { review(&clips, study, model, args.get(5).is_some_and(|s| s == "history-only")) }
            else { let phase = args.get(5).ok_or("baseline or learned required")?;
                if !["baseline", "learned"].contains(&phase.as_str()) { return Err("Unknown phase".into()); }
                student(&clips, study, model, phase) }
        }
        Some("history-audit") => {
            let clips: Vec<Clip> = read(Path::new(args.get(2).ok_or("manifest required")?))?;
            history_audit(&clips, Path::new(args.get(3).ok_or("native study required")?), Path::new(args.get(4).ok_or("output directory required")?))
        }
        Some("cleanup-study") => {
            let clips: Vec<Clip> = read(Path::new(args.get(2).ok_or("manifest required")?))?;
            cleanup_study(&clips, Path::new(args.get(3).ok_or("native study required")?), Path::new(args.get(4).ok_or("output directory required")?))
        }
        Some("quiet-probe") => quiet_probe(args.get(2).ok_or("model required")?,args.get(3).ok_or("audio required")?,Path::new(args.get(4).ok_or("output required")?)),
        Some("example-probe") => example_probe(Path::new(args.get(2).ok_or("study required")?),Path::new(args.get(3).ok_or("output required")?)),
        Some("cleanup-probe") => {
            let clips: Vec<Clip>=read(Path::new(args.get(2).ok_or("manifest required")?))?;
            cleanup_probe(&clips,Path::new(args.get(3).ok_or("native study required")?),Path::new(args.get(4).ok_or("learned study required")?),Path::new(args.get(5).ok_or("output required")?))
        }
        Some("quiet-examples") => quiet::examples(
            Path::new(args.get(2).ok_or("manifest required")?),
            Path::new(args.get(3).ok_or("output required")?),
            args.get(4).ok_or("model required")?,
            &args[5..],
        ),
        Some("quiet") => quiet::run(
            Path::new(args.get(2).ok_or("manifest required")?),
            Path::new(args.get(3).ok_or("output required")?),
            args.get(4).ok_or("model required")?,
            args.get(5).is_none_or(|flag| flag != "no-asr"),
        ),
        Some("format") => formatting(Path::new(args.get(2).ok_or("cases required")?), Path::new(args.get(3).ok_or("output required")?), args.get(4).is_some_and(|s| s == "llm"), args.get(5).map_or(Ok(6), |s| s.parse::<u64>().map_err(|e|e.to_string()))?),
        _ => Err("Usage: speechify-bench download MODEL | student MANIFEST STUDY MODEL baseline|learned | teacher MANIFEST STUDY MODEL [history-only] | format CASES OUTPUT [llm] [timeout-seconds] | cleanup-study MANIFEST NATIVE_STUDY OUTPUT_DIR | quiet MANIFEST OUTPUT MODEL [no-asr] | quiet-examples MANIFEST OUTPUT MODEL ID... | quiet-probe MODEL AUDIO OUTPUT | example-probe STUDY OUTPUT | cleanup-probe MANIFEST NATIVE_STUDY LEARNED_STUDY OUTPUT".into()),
    }
}
