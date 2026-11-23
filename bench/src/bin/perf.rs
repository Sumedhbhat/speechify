use serde::Deserialize;
use serde_json::{json, Value};
use speechify_core::{
    audio,
    context::{Category, Target},
    diff,
    llm::{Chat, Rewriter},
    native::{Detector, Engine, DETECTOR_FILE},
    pipeline::{self, Outcome, Request, RewriteCircuit},
    settings::Settings,
    store::{Clock, Store},
    Result,
};
use std::{
    cell::RefCell,
    env, fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const JFK: &str =
    "And so my fellow Americans, ask not what your country can do for you, ask what you can do for your country.";
const RUNS: usize = 5;
const RULE_PASSES: usize = 50;
const CLOCK: Clock = Clock {
    now: 1_794_355_200,
    offset_min: 330,
};

#[derive(Deserialize)]
struct Scenario {
    id: String,
    kind: String,
    raw: String,
    #[serde(default)]
    category: String,
    checks: Checks,
}

#[derive(Deserialize, Default)]
struct Checks {
    equals: Option<String>,
    #[serde(default)]
    contains: Vec<Value>,
    #[serde(default)]
    absent: Vec<String>,
    list: Option<String>,
    items: Option<usize>,
    paragraphs: Option<usize>,
    min_lines: Option<usize>,
}

struct Capture<'a> {
    inner: &'a Chat,
    calls: RefCell<Vec<(f64, String)>>,
}

impl Rewriter for Capture<'_> {
    fn rewrite(&self, system: &str, user: &str) -> Result<String> {
        let start = Instant::now();
        let answer = self.inner.rewrite(system, user);
        let text = answer.as_ref().map_or_else(|e| format!("error: {e}"), Clone::clone);
        self.calls.borrow_mut().push((ms(start), text));
        answer
    }
    fn model(&self) -> &str {
        self.inner.model()
    }
}

fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

fn median(values: &[f64]) -> f64 {
    percentile(values, 0.5)
}

fn percentile(values: &[f64], at: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = ((sorted.len() as f64 * at).ceil() as usize).clamp(1, sorted.len());
    sorted[rank - 1]
}

fn env_path(name: &str) -> Result<PathBuf> {
    env::var_os(name).map(PathBuf::from).ok_or(format!("{name} is not set"))
}

fn settings() -> Settings {
    let mut s = Settings::default();
    s.speech.language = "en".into();
    s.speech.device = "cpu".into();
    s.rewrite.enabled = false;
    s
}

fn destination(category: &str) -> (Target, Category) {
    let (app, category) = match category {
        "email" => ("Gmail", Category::Email),
        "code" => ("Visual Studio Code", Category::Code),
        "personal" => ("WhatsApp", Category::Personal),
        _ => ("Slack", Category::Work),
    };
    (
        Target {
            app: app.into(),
            ..Target::default()
        },
        category,
    )
}

fn speech(runtime: &Path, model: &Path) -> Result<Value> {
    let pcm = audio::decode_wav(include_bytes!("../../../core/tests/fixtures/jfk.wav"))?;
    let start = Instant::now();
    let engine = Engine::open(runtime, model, "cpu")?;
    let load_ms = ms(start);
    let detector = Detector::open(runtime, &runtime.join(DETECTOR_FILE), "cpu")?;
    let mut engine = engine.with_detector(detector);
    let settings = settings();
    let store = Store::memory();
    let request = Request {
        audio: &pcm,
        target: destination("").0,
        command: false,
        selection: "",
        clock: CLOCK,
    };
    let mut once = || -> Result<(f64, String)> {
        let start = Instant::now();
        let outcome = pipeline::run(&request, &settings, &store, &mut engine, None)?;
        let elapsed = ms(start);
        match outcome {
            Outcome::Text { paste, .. } => Ok((elapsed, paste)),
            other => Err(format!("The test clip gave {other:?}")),
        }
    };
    let (first_ms, text) = once()?;
    let mut runs = vec![];
    for _ in 0..RUNS {
        runs.push(once()?.0);
    }
    let audio_s = pcm.len() as f64 / 16_000.0;
    let dictation_ms = median(&runs);
    Ok(json!({
        "model": model.file_stem().and_then(|s| s.to_str()).unwrap_or_default(),
        "load_ms": load_ms,
        "first_ms": first_ms,
        "dictation_ms": dictation_ms,
        "runs_ms": runs,
        "audio_s": audio_s,
        "realtime_factor": dictation_ms / 1000.0 / audio_s,
        "wer": diff::wer(JFK, &text),
        "text": text,
    }))
}

fn find(text: &str, phrase: &str) -> bool {
    let (text, phrase) = (text.to_lowercase(), phrase.to_lowercase());
    let edge = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric());
    let starts = phrase.chars().next().is_some_and(char::is_alphanumeric);
    let ends = phrase.chars().last().is_some_and(char::is_alphanumeric);
    text.match_indices(&phrase).any(|(at, _)| {
        (!starts || edge(text[..at].chars().last())) && (!ends || edge(text[at + phrase.len()..].chars().next()))
    })
}

fn numbered(line: &str) -> bool {
    let line = line.trim_start();
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    digits > 0 && line[digits..].starts_with(['.', ')']) && line[digits + 1..].starts_with(' ')
}

fn bulleted(line: &str) -> bool {
    let line = line.trim_start();
    ["- ", "* ", "• "].iter().any(|mark| line.starts_with(mark))
}

fn failures(text: &str, checks: &Checks) -> Vec<String> {
    let mut failed = vec![];
    if let Some(expected) = &checks.equals {
        if text != expected {
            failed.push(format!("expected exactly {expected:?}"));
        }
    }
    for wanted in &checks.contains {
        let options: Vec<&str> = match wanted {
            Value::Array(items) => items.iter().filter_map(Value::as_str).collect(),
            other => other.as_str().into_iter().collect(),
        };
        if !options
            .iter()
            .any(|option| text.to_lowercase().contains(&option.to_lowercase()))
        {
            failed.push(format!("missing {}", options.join(" or ")));
        }
    }
    for unwanted in &checks.absent {
        if find(text, unwanted) {
            failed.push(format!("still has {unwanted:?}"));
        }
    }
    let lines: Vec<&str> = text.lines().filter(|line| !line.trim().is_empty()).collect();
    let numbers = lines.iter().filter(|line| numbered(line)).count();
    let bullets = lines.iter().filter(|line| bulleted(line)).count();
    if let Some(list) = &checks.list {
        let (found, count) = match (numbers, bullets) {
            (0, 0) => ("none", 0),
            (n, 0) => ("numbered", n),
            (0, b) => ("bullets", b),
            (n, b) => ("mixed", n + b),
        };
        if found != list {
            failed.push(format!("expected {list} list, got {found}"));
        } else if let Some(items) = checks.items.filter(|items| *items != count) {
            failed.push(format!("expected {items} items, got {count}"));
        }
    }
    if let Some(paragraphs) = checks.paragraphs {
        let found = text.split("\n\n").filter(|block| !block.trim().is_empty()).count();
        if found != paragraphs {
            failed.push(format!("expected {paragraphs} paragraphs, got {found}"));
        }
    }
    if let Some(min) = checks.min_lines.filter(|min| lines.len() < *min) {
        failed.push(format!("expected at least {min} lines, got {}", lines.len()));
    }
    failed
}

fn rules(scenarios: &[Scenario]) -> (Value, Vec<Value>) {
    let settings = settings();
    let store = Store::memory();
    let compose = |s: &Scenario| {
        let (target, category) = destination(&s.category);
        pipeline::compose(&s.raw, &target, category, &settings, &store, None).map_or(String::new(), |c| c.text)
    };
    let mut passes = vec![];
    for _ in 0..RULE_PASSES {
        let start = Instant::now();
        for s in scenarios {
            std::hint::black_box(compose(s));
        }
        passes.push(ms(start));
    }
    let rows: Vec<Value> = scenarios
        .iter()
        .map(|s| {
            let text = compose(s);
            let failed = failures(&text, &s.checks);
            json!({"text": text, "passed": failed.is_empty(), "failures": failed})
        })
        .collect();
    let passed = rows.iter().filter(|r| r["passed"] == true).count();
    (
        json!({"pass_ms": median(&passes), "passed": passed, "total": rows.len()}),
        rows,
    )
}

fn language_model(scenarios: &[Scenario], url: &str, model: &str) -> (Value, Vec<Value>) {
    let chat = Chat::new(url, "", model, Duration::from_secs(180));
    let capture = Capture {
        inner: &chat,
        calls: RefCell::new(vec![]),
    };
    let mut settings = settings();
    settings.rewrite.enabled = true;
    settings.rewrite.model = model.into();
    let store = Store::memory();
    let compose = |s: &Scenario| {
        let (target, category) = destination(&s.category);
        pipeline::compose_guarded(
            &s.raw,
            &target,
            category,
            &settings,
            &store,
            Some(&capture as &dyn Rewriter),
            (&mut RewriteCircuit::default(), 0),
        )
    };
    if let Some(first) = scenarios.iter().max_by_key(|s| s.raw.split_whitespace().count()) {
        compose(first);
    }
    capture.calls.take();
    let mut latencies = vec![];
    let rows: Vec<Value> = scenarios
        .iter()
        .map(|s| {
            let out = compose(s);
            let calls = capture.calls.take();
            latencies.extend(calls.iter().map(|(ms, _)| *ms));
            let text = out.as_ref().map_or(String::new(), |c| c.text.clone());
            let failed = failures(&text, &s.checks);
            eprintln!("llm {} {}", s.id, if failed.is_empty() { "pass" } else { "fail" });
            json!({
                "text": text,
                "passed": failed.is_empty(),
                "failures": failed,
                "ms": calls.iter().map(|(ms, _)| ms).sum::<f64>(),
                "rewritten": out.as_ref().is_some_and(|c| c.rewritten),
                "fallback": out.and_then(|c| c.fallback),
                "answer": calls.last().map(|(_, answer)| answer.clone()),
            })
        })
        .collect();
    let passed = rows.iter().filter(|r| r["passed"] == true).count();
    let summary = json!({
        "model": model,
        "median_ms": median(&latencies),
        "p90_ms": percentile(&latencies, 0.9),
        "calls": latencies.len(),
        "rewritten": rows.iter().filter(|r| r["rewritten"] == true).count(),
        "fallbacks": rows.iter().filter(|r| !r["fallback"].is_null()).count(),
        "passed": passed,
        "total": rows.len(),
    });
    (summary, rows)
}

fn run() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let (Some(cases), Some(output)) = (args.get(1), args.get(2)) else {
        return Err("Usage: perf SCENARIOS OUTPUT".into());
    };
    let scenarios: Vec<Scenario> =
        serde_json::from_slice(&fs::read(cases).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let runtime = env_path("SPEECHIFY_NATIVE_DIR")?;
    let mut speech_rows = vec![];
    for name in ["SPEECHIFY_TEST_MODEL", "SPEECHIFY_TEST_TOKEN_MODEL"] {
        let row = speech(&runtime, &env_path(name)?)?;
        eprintln!("speech {} {:.0} ms", row["model"], row["dictation_ms"]);
        speech_rows.push(row);
    }
    let (rules_summary, rule_rows) = rules(&scenarios);
    let llm = match (env::var("PERF_LLM_URL"), env::var("PERF_LLM_MODEL")) {
        (Ok(url), Ok(model)) if !url.is_empty() => Some(language_model(&scenarios, &url, &model)),
        _ => None,
    };
    let rows: Vec<Value> = scenarios
        .iter()
        .enumerate()
        .map(|(i, s)| {
            json!({
                "id": s.id,
                "kind": s.kind,
                "raw": s.raw,
                "rules": rule_rows[i],
                "llm": llm.as_ref().map(|(_, rows)| rows[i].clone()),
            })
        })
        .collect();
    let image = format!(
        "{} {}",
        env::var("ImageOS").unwrap_or_default(),
        env::var("ImageVersion").unwrap_or_default()
    );
    let report = json!({
        "schema": 1,
        "platform": env::var("PERF_PLATFORM").unwrap_or_else(|_| env::consts::OS.into()),
        "arch": env::consts::ARCH,
        "cpus": std::thread::available_parallelism().map_or(0, |n| n.get()),
        "image": image.trim(),
        "speech": speech_rows,
        "rules": rules_summary,
        "llm": llm.as_ref().map(|(summary, _)| summary),
        "scenarios": rows,
    });
    fs::write(output, serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
