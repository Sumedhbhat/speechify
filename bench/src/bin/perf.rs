use serde::Deserialize;
use serde_json::{json, Value};
use speechify_core::{
    audio, catalog,
    context::{Category, Target},
    diff,
    llm::{Chat, Rewriter},
    native::{Detector, Engine, DETECTOR_FILE},
    pipeline::{self, Outcome, Request, RewriteCircuit},
    settings::{Settings, DEFAULTS},
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
const SCHEMA: u32 = 2;
const RUNS: usize = 3;
const RULE_PASSES: usize = 50;
const CLOCK: Clock = Clock {
    now: 1_794_355_200,
    offset_min: 330,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    id: String,
    kind: String,
    raw: String,
    #[serde(default)]
    category: String,
    checks: Checks,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
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

fn transcription(runtime: &Path, model: &Path) -> Result<Value> {
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
            Outcome::Text { record, .. } => Ok((elapsed, record.raw)),
            other => Err(format!("The test clip gave {other:?}")),
        }
    };
    let (first_ms, text) = once()?;
    let mut runs = vec![];
    for _ in 0..RUNS {
        runs.push(once()?.0);
    }
    let audio_s = pcm.len() as f64 / 16_000.0;
    let time_ms = median(&runs);
    Ok(json!({
        "load_ms": load_ms,
        "first_ms": first_ms,
        "time_ms": time_ms,
        "runs_ms": runs,
        "audio_s": audio_s,
        "realtime": audio_s * 1000.0 / time_ms,
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
        if !options.iter().any(|option| find(text, option)) {
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

fn validate(scenarios: &[Scenario]) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    for s in scenarios {
        let problem = if !seen.insert(s.id.as_str()) {
            Some("appears twice".to_string())
        } else if !["", "work", "email", "code", "personal"].contains(&s.category.as_str()) {
            Some(format!("has an unknown category {:?}", s.category))
        } else {
            checks_problem(&s.checks)
        };
        if let Some(problem) = problem {
            return Err(format!("Scenario {} {problem}", s.id));
        }
    }
    Ok(())
}

fn checks_problem(checks: &Checks) -> Option<String> {
    let phrases = |v: &Value| match v {
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty() && items.iter().all(|i| i.as_str().is_some_and(|t| !t.is_empty())),
        _ => false,
    };
    if checks.items.is_some() && checks.list.as_deref().is_none_or(|list| list == "none") {
        Some("sets items without a numbered or bullet list".into())
    } else if checks.equals.is_none()
        && checks.contains.is_empty()
        && checks.absent.is_empty()
        && checks.list.is_none()
        && checks.paragraphs.is_none()
        && checks.min_lines.is_none()
    {
        Some("has no checks".into())
    } else if checks
        .list
        .as_deref()
        .is_some_and(|l| !["numbered", "bullets", "none"].contains(&l))
    {
        Some("has a list that is not numbered, bullets or none".into())
    } else if !checks.contains.iter().all(phrases) {
        Some("has a contains entry that is not a phrase or a list of phrases".into())
    } else if checks.absent.iter().any(String::is_empty) {
        Some("has an empty absent phrase".into())
    } else {
        None
    }
}

fn rules(scenarios: &[Scenario]) -> (f64, Vec<Value>) {
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
    (median(&passes), rows)
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
    settings.rewrite.min_words = 0;
    let store = Store::memory();
    let compose = |s: &Scenario| {
        let (target, category) = destination(&s.category);
        let mut circuit = RewriteCircuit::default();
        let out = pipeline::compose_guarded(
            &s.raw,
            &target,
            category,
            &settings,
            &store,
            Some(&capture as &dyn Rewriter),
            (&mut circuit, 0),
        );
        (out, circuit.failures() > 0)
    };
    if let Some(first) = scenarios.iter().max_by_key(|s| s.raw.split_whitespace().count()) {
        compose(first);
    }
    capture.calls.take();
    let mut latencies = vec![];
    let rows: Vec<Value> = scenarios
        .iter()
        .map(|s| {
            let (out, rejected) = compose(s);
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
                "rejected": rejected,
                "answer": calls.last().map(|(_, answer)| answer.clone()),
            })
        })
        .collect();
    let summary = json!({
        "median_ms": median(&latencies),
        "p90_ms": percentile(&latencies, 0.9),
        "calls": latencies.len(),
        "rejected": rows.iter().filter(|r| r["rejected"] == true).count(),
    });
    (summary, rows)
}

fn run() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let (Some(cases), Some(output)) = (args.get(1), args.get(2)) else {
        return Err("Usage: perf SCENARIOS OUTPUT".into());
    };
    let scenarios: Vec<Scenario> =
        serde_json::from_slice(&fs::read(cases).map_err(|e| e.to_string())?).map_err(|e| format!("{cases}: {e}"))?;
    validate(&scenarios)?;
    let runtime = env_path("SPEECHIFY_NATIVE_DIR")?;
    let speech = catalog::find(&DEFAULTS.speech_model)?;
    let models_dir = env::var_os("PERF_MODELS_DIR").map_or_else(|| PathBuf::from(".build/models"), PathBuf::from);
    let speech_path = if catalog::installed(&models_dir, &speech) {
        catalog::path(&models_dir, &speech)
    } else {
        eprintln!("downloading {}", speech.id);
        catalog::download(&models_dir, &speech, &mut |_, _| true)?
    };
    let heard = transcription(&runtime, &speech_path)?;
    eprintln!("transcription {:.0} ms", heard["time_ms"]);
    let (rules_ms, rule_rows) = rules(&scenarios);
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
        "schema": SCHEMA,
        "platform": env::var("PERF_PLATFORM").unwrap_or_else(|_| env::consts::OS.into()),
        "arch": env::consts::ARCH,
        "cpus": std::thread::available_parallelism().map_or(0, |n| n.get()),
        "image": image.trim(),
        "models": {
            "speech": speech.id,
            "cleanup": env::var("PERF_LLM_MODEL").ok().filter(|_| llm.is_some()),
            "cleanup_digest": env::var("PERF_LLM_DIGEST").ok().filter(|_| llm.is_some()),
        },
        "transcription": heard,
        "cleanup": {
            "rules_ms": rules_ms,
            "llm": llm.as_ref().map(|(summary, _)| summary),
        },
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

#[cfg(test)]
mod tests {
    use super::*;

    fn checks(value: Value) -> Checks {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn find_matches_whole_words_only() {
        assert!(find("Meet at 3 tomorrow", "3"));
        assert!(!find("Meet at 13 tomorrow", "3"));
        assert!(find("Hi Priya, thanks", "hi priya,"));
        assert!(find("Ship it?", "?"));
        assert!(!find("the latest master branch", "latest branch"));
        assert!(find("git push --force", "--force"));
    }

    #[test]
    fn lists_are_told_apart() {
        assert!(numbered("1. Buy milk"));
        assert!(numbered("  12) Pay"));
        assert!(!numbered("1.5 litres"));
        assert!(!numbered("2024 was good"));
        assert!(bulleted("- tea"));
        assert!(bulleted("• rice"));
        assert!(!bulleted("-5 degrees"));
    }

    #[test]
    fn list_checks_count_items_and_reject_mixed_lists() {
        let numbered_three = checks(json!({"list": "numbered", "items": 3}));
        assert!(failures("Plan:\n1. A\n2. B\n3. C", &numbered_three).is_empty());
        assert_eq!(failures("1. A\n2. B", &numbered_three), ["expected 3 items, got 2"]);
        assert_eq!(
            failures("1. A\n- B\n2. C", &numbered_three),
            ["expected numbered list, got mixed"]
        );
        assert_eq!(
            failures("A, B and C.", &numbered_three),
            ["expected numbered list, got none"]
        );
        assert!(failures("Just prose.", &checks(json!({"list": "none"}))).is_empty());
    }

    #[test]
    fn text_checks_report_every_failure() {
        let c = checks(json!({"contains": [["six", "6"], "table"], "absent": ["four"]}));
        assert!(failures("Book a table for 6.", &c).is_empty());
        assert_eq!(
            failures("Book four seats.", &c),
            ["missing six or 6", "missing table", "still has \"four\""]
        );
        assert_eq!(
            failures("Okay.", &checks(json!({"equals": "Okay"}))),
            ["expected exactly \"Okay\""]
        );
    }

    #[test]
    fn paragraphs_and_lines_are_counted() {
        let c = checks(json!({"paragraphs": 3, "min_lines": 3}));
        assert!(failures("Hi team\n\nThe build is green.\n\nThanks", &c).is_empty());
        assert_eq!(
            failures("Hi team\nThanks", &c),
            ["expected 3 paragraphs, got 1", "expected at least 3 lines, got 2"]
        );
    }

    #[test]
    fn scenarios_with_mistakes_are_refused() {
        let scenario = |checks: Value| -> std::result::Result<Scenario, serde_json::Error> {
            serde_json::from_value(json!({"id": "a", "kind": "lists", "raw": "x", "checks": checks}))
        };
        assert!(scenario(json!({"absnt": ["x"]})).is_err());
        let refused = |checks: Value| validate(&[scenario(checks).unwrap()]).unwrap_err();
        assert!(refused(json!({})).contains("no checks"));
        assert!(refused(json!({"items": 3})).contains("items without"));
        assert!(refused(json!({"list": "none", "items": 2})).contains("items without"));
        assert!(refused(json!({"list": "bullet"})).contains("not numbered"));
        assert!(refused(json!({"contains": [3]})).contains("contains entry"));
        assert!(refused(json!({"contains": [[]]})).contains("contains entry"));
        assert!(refused(json!({"absent": [""]})).contains("empty absent"));
        let twice = [
            scenario(json!({"list": "none"})).unwrap(),
            scenario(json!({"list": "none"})).unwrap(),
        ];
        assert!(validate(&twice).unwrap_err().contains("appears twice"));
        let mut odd = scenario(json!({"list": "none"})).unwrap();
        odd.category = "chat".into();
        assert!(validate(&[odd]).unwrap_err().contains("unknown category"));
    }

    #[test]
    fn every_scenario_in_the_suite_is_valid() {
        let scenarios: Vec<Scenario> = serde_json::from_str(include_str!("../../scenarios.json")).unwrap();
        validate(&scenarios).unwrap();
        assert!(scenarios.len() >= 30);
    }

    #[test]
    fn percentiles_pick_the_nearest_rank() {
        assert_eq!(median(&[]), 0.0);
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(
            percentile(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0], 0.9),
            9.0
        );
    }
}
