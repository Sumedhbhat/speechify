//! One dictation from audio to text: trim, recognise, clean up, record.

use crate::asr::{is_phantom, strip_tags, Hint, Recognizer};
use crate::context::{categorize, Category, Target};
use crate::format::{self, Edits, Options};
use crate::llm::{cleanup_prompt, command_prompt, vet_cleanup, vet_command, Brief, Rewriter};
use crate::settings::{Cleanup, Settings};
use crate::store::{Clock, Dictation, Store};
use crate::{audio, Result};
use regex::Regex;
use std::sync::LazyLock;
use std::time::Instant;

static PRESS_ENTER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)(^|[\s,.;:]+)press enter[\s.!]*$").unwrap());

pub struct Request<'a> {
    /// 16 kHz mono recording, untrimmed.
    pub audio: &'a [f32],
    pub target: Target,
    /// Command Mode: the speech is an instruction for `selection`.
    pub command: bool,
    pub selection: &'a str,
    pub clock: Clock,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Text to paste, the record to save, and whether to press Enter after.
    Text {
        paste: String,
        record: Box<Dictation>,
        press_enter: bool,
    },
    /// The recording had no speech in it: probably a muted microphone.
    Silent,
    /// Something was heard but it amounted to nothing (noise, fillers).
    Nothing,
}

/// The cleaned-up form of something that was heard.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Composed {
    pub text: String,
    pub rewritten: bool,
    pub fixes: u32,
    pub press_enter: bool,
    pub rewrite_ms: u32,
    /// Why the language model was not used, if it was meant to be.
    pub fallback: Option<String>,
}

/// Back off after two failed rewrites. The controller supplies monotonic time;
/// rules remain available immediately while the language model cools down.
#[derive(Debug, Default)]
pub struct RewriteCircuit {
    failures: u32,
    pub retry_at_ms: u64,
}

impl RewriteCircuit {
    pub fn cooling(&self, now_ms: u64) -> bool {
        now_ms < self.retry_at_ms
    }

    fn record(&mut self, now_ms: u64, success: bool) {
        if success {
            self.failures = 0;
            self.retry_at_ms = 0;
        } else {
            self.failures = self.failures.saturating_add(1);
            if self.failures >= 2 {
                self.retry_at_ms = now_ms.saturating_add(60_000);
            }
        }
    }
}

/// Turns recognised words into the text to type. `None` means nothing was said.
pub fn compose(
    raw: &str,
    target: &Target,
    category: Category,
    settings: &Settings,
    store: &Store,
    rewriter: Option<&dyn Rewriter>,
) -> Option<Composed> {
    compose_guarded(
        raw,
        target,
        category,
        settings,
        store,
        rewriter,
        (&mut RewriteCircuit::default(), 0),
    )
}

/// Composition with a persistent rewrite breaker and injected monotonic time.
pub fn compose_guarded(
    raw: &str,
    target: &Target,
    category: Category,
    settings: &Settings,
    store: &Store,
    rewriter: Option<&dyn Rewriter>,
    (circuit, now_ms): (&mut RewriteCircuit, u64),
) -> Option<Composed> {
    let mut out = Composed::default();
    let mut heard = raw.trim().to_string();
    if settings.press_enter && PRESS_ENTER.is_match(&heard) {
        heard = PRESS_ENTER.replace(&heard, "").trim().to_string();
        out.press_enter = true;
    }
    if heard.is_empty() {
        // "Press enter" alone still presses Enter.
        return out.press_enter.then_some(out);
    }
    let rules = store.replacement_rules();
    let options = Options {
        style: category.style(&settings.styles),
        spelling: settings.spelling,
        numbers: settings.numbers,
        replacements: &rules.replacements,
        literal: category.literal(),
    };
    if settings.rewrite.level == Cleanup::None {
        // Raw mode still honours the user's own dictionary and snippets.
        let (text, fired) = format::apply_replacements(&heard, &rules.replacements);
        let _ = store.record_hits(&rules, &fired);
        out.fixes = fired.len() as u32;
        out.text = text;
        return Some(out);
    }
    let by_rules = format::format(&heard, &options);
    let mut result = by_rules.clone();
    let long_enough = heard.split_whitespace().count() >= settings.rewrite.min_words;
    if let (true, true, Some(rewriter)) = (settings.rewrite.enabled, long_enough, rewriter) {
        if circuit.cooling(now_ms) {
            out.text = by_rules.text;
            out.fixes = by_rules.edits.total();
            out.fallback = Some("Language model temporarily skipped after repeated rewrite failures".into());
            let _ = store.record_hits(&rules, &by_rules.fired);
            return (!out.text.is_empty() || out.press_enter).then_some(out);
        }
        let vocabulary = store.vocabulary();
        let examples = store.examples(category, 3);
        let brief = Brief {
            level: settings.rewrite.level,
            style: options.style,
            category,
            spelling: settings.spelling,
            numbers: settings.numbers,
            target,
            vocabulary: &vocabulary,
            examples: &examples,
        };
        let (system, user) = cleanup_prompt(&heard, &brief);
        let started = Instant::now();
        let answer = rewriter.rewrite(&system, &user);
        out.rewrite_ms = started.elapsed().as_millis() as u32;
        let vetted = answer.map(|text| vet_cleanup(&heard, &text));
        circuit.record(now_ms, matches!(vetted, Ok(Some(_))));
        match vetted {
            Ok(Some(text)) if text.is_empty() => return None,
            Ok(Some(text)) => {
                // The model did the rewriting; the rule pass's count is the best
                // estimate of how much it fixed.
                result = format::finish(
                    &text,
                    &options,
                    Edits {
                        replacements: 0,
                        spellings: 0,
                        ..by_rules.edits
                    },
                );
                out.rewritten = true;
            }
            Ok(None) => {
                out.fallback = Some(
                    "The language model's answer did not look like a cleanup, so the built-in formatter was used"
                        .into(),
                )
            }
            Err(error) => out.fallback = Some(error),
        }
    }
    let _ = store.record_hits(&rules, &result.fired);
    out.fixes = result.edits.total();
    out.text = result.text;
    (!out.text.is_empty() || out.press_enter).then_some(out)
}

/// What is pasted: a trailing space lets the next dictation continue the
/// line, except in code and terminals where it would be typed literally.
fn paste_form(text: &str, category: Category) -> String {
    if text.is_empty() || category.literal() || text.contains('\n') {
        text.to_string()
    } else {
        format!("{text} ")
    }
}

pub fn run(
    request: &Request,
    settings: &Settings,
    store: &Store,
    recognizer: &mut dyn Recognizer,
    rewriter: Option<&dyn Rewriter>,
) -> Result<Outcome> {
    run_guarded(
        request,
        settings,
        store,
        recognizer,
        rewriter,
        (&mut RewriteCircuit::default(), 0),
    )
}

/// Runs a dictation with the controller's persistent rewrite circuit.
pub fn run_guarded(
    request: &Request,
    settings: &Settings,
    store: &Store,
    recognizer: &mut dyn Recognizer,
    rewriter: Option<&dyn Rewriter>,
    circuit: (&mut RewriteCircuit, u64),
) -> Result<Outcome> {
    let started = Instant::now();
    let trimmed = audio::trim(request.audio);
    if trimmed.samples.is_empty() {
        return Ok(Outcome::Silent);
    }
    let vocabulary = store.vocabulary();
    let raw = strip_tags(&recognizer.transcribe(
        &trimmed.samples,
        &Hint {
            language: &settings.speech.language,
            vocabulary: &vocabulary,
        },
    )?);
    let asr_ms = started.elapsed().as_millis() as u32;
    if raw.is_empty() || is_phantom(&raw, trimmed.speech_ms) {
        return Ok(Outcome::Nothing);
    }
    let category = categorize(&request.target, &store.rules());
    let mut record = Dictation {
        created_at: request.clock.now,
        offset_min: request.clock.offset_min,
        app: request.target.app.clone(),
        process: request.target.process.clone(),
        title: request.target.title.clone(),
        category,
        command: request.command,
        duration_ms: (request.audio.len() as u64 * 1000 / audio::RATE as u64) as u32,
        speech_ms: trimmed.speech_ms as u32,
        raw: raw.clone(),
        speech_model: recognizer.name().to_string(),
        asr_ms,
        ..Dictation::default()
    };
    let mut press_enter = false;
    if request.command {
        let rewriter = rewriter.ok_or("Command Mode needs the language model. Turn it on in Settings.")?;
        let (system, user) = command_prompt(request.selection, &raw, &vocabulary);
        let asked = Instant::now();
        let answer = rewriter.rewrite(&system, &user)?;
        record.rewrite_ms = asked.elapsed().as_millis() as u32;
        record.text = vet_command(&answer).ok_or("The language model returned nothing for that command")?;
        record.rewritten = true;
    } else {
        let Some(composed) = compose_guarded(&raw, &request.target, category, settings, store, rewriter, circuit)
        else {
            return Ok(Outcome::Nothing);
        };
        record.text = composed.text;
        record.rewritten = composed.rewritten;
        record.rewrite_ms = composed.rewrite_ms;
        record.fixes = composed.fixes;
        press_enter = composed.press_enter;
    }
    if record.rewritten {
        record.rewrite_model = rewriter.map(|r| r.model().to_string()).unwrap_or_default();
    }
    record.words = record.text.split_whitespace().count() as u32;
    record.total_ms = started.elapsed().as_millis() as u32;
    let paste = if request.command { record.text.clone() } else { paste_form(&record.text, category) };
    Ok(Outcome::Text {
        paste,
        record: Box::new(record),
        press_enter,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asr::Fixed;
    use crate::audio::tone;
    use crate::llm::Scripted;
    use crate::settings::Style;

    const NOW: Clock = Clock {
        now: 1_800_000_000,
        offset_min: 330,
    };

    fn request<'a>(audio: &'a [f32], app: &str) -> Request<'a> {
        Request {
            audio,
            target: Target {
                app: app.into(),
                process: String::new(),
                title: String::new(),
            },
            command: false,
            selection: "",
            clock: NOW,
        }
    }

    // Kept on one line so that the line that can only run when a test is
    // about to fail is not counted as unexecuted code.
    #[rustfmt::skip]
    fn text(outcome: Result<Outcome>) -> (String, Dictation, bool) {
        let outcome = outcome.unwrap();
        let shown = format!("{outcome:?}");
        let Outcome::Text { paste, record, press_enter } = outcome else { panic!("not text: {shown}") };
        (paste, *record, press_enter)
    }

    #[test]
    fn rules_only_dictation_into_a_chat_app() {
        let store = Store::memory();
        store.add_word("sumit", "Sumedh", "manual", NOW).unwrap();
        let mut settings = Settings::default();
        settings.rewrite.enabled = false;
        let mut asr = Fixed::new(&["um tell sumit the color is fine"]);
        let audio = tone(2000, 0.3);
        let (paste, record, enter) = text(run(&request(&audio, "WhatsApp"), &settings, &store, &mut asr, None));
        assert_eq!(paste, "Tell Sumedh the colour is fine ");
        assert_eq!(record.text, "Tell Sumedh the colour is fine");
        assert_eq!(
            (record.category, record.words, record.rewritten, record.fixes),
            (Category::Personal, 6, false, 3)
        );
        assert_eq!(
            (record.duration_ms, record.speech_ms, record.created_at),
            (2000, 2000, NOW.now)
        );
        assert_eq!(record.speech_model, "fixed");
        assert!(!enter);
        assert_eq!(asr.heard[0].1, "en");
        assert_eq!(asr.heard[0].2, vec!["Sumedh"]);
        assert_eq!(store.dictionary().unwrap()[0].hits, 1);
    }

    #[test]
    fn silence_noise_and_phantoms_produce_nothing() {
        let store = Store::memory();
        let settings = Settings::default();
        let mut asr = Fixed::new(&["[BLANK_AUDIO]", "Thank you.", "um uh"]);
        assert_eq!(
            run(&request(&[0.0; 16_000], "A"), &settings, &store, &mut asr, None).unwrap(),
            Outcome::Silent
        );
        assert!(asr.heard.is_empty());
        let short = tone(300, 0.3);
        assert_eq!(
            run(&request(&short, "A"), &settings, &store, &mut asr, None).unwrap(),
            Outcome::Nothing
        );
        assert_eq!(
            run(&request(&short, "A"), &settings, &store, &mut asr, None).unwrap(),
            Outcome::Nothing
        );
        assert_eq!(
            run(&request(&tone(2000, 0.3), "A"), &settings, &store, &mut asr, None).unwrap(),
            Outcome::Nothing
        );
        let mut failing = Fixed {
            answers: vec![Err("engine down".into())],
            heard: vec![],
        };
        assert_eq!(
            run(&request(&short, "A"), &settings, &store, &mut failing, None).unwrap_err(),
            "engine down"
        );
    }

    #[test]
    fn the_language_model_rewrites_and_rules_finish() {
        let store = Store::memory();
        store
            .add_example(Category::Work, "uh ping me", "Ping me.", NOW)
            .unwrap();
        let settings = Settings::default();
        let mut asr = Fixed::new(&["so um the organization needs two things one a budget two a color scheme"]);
        let llm = Scripted::new(vec![Ok(
            "The organization needs two things:\n1. A budget\n2. A color scheme".into(),
        )]);
        let audio = tone(3000, 0.3);
        let (paste, record, _) = text(run(&request(&audio, "Slack"), &settings, &store, &mut asr, Some(&llm)));
        assert_eq!(
            paste,
            "The organisation needs two things:\n1. A budget\n2. A colour scheme"
        );
        assert!(record.rewritten);
        assert_eq!(record.rewrite_model, "scripted");
        assert!(record.fixes >= 3);
        let seen = llm.seen.borrow();
        assert!(seen[0].0.contains("Destination: Work messages") && seen[0].0.contains("RAW: uh ping me"));
        assert!(seen[0].1.contains("typing into Slack"));
    }

    #[test]
    fn a_bad_or_failed_model_answer_falls_back_to_rules() {
        let store = Store::memory();
        let settings = Settings::default();
        let target = Target::default();
        let raw = "what is the capital of france please tell me";
        let llm = Scripted::new(vec![
            Ok("The capital of France is Paris, known for the Eiffel Tower.".into()),
            Err("Could not reach 127.0.0.1:11434. Is it running?".into()),
            Ok("EMPTY".into()),
        ]);
        let off_rails = compose(raw, &target, Category::Other, &settings, &store, Some(&llm)).unwrap();
        assert_eq!(off_rails.text, "What is the capital of france please tell me.");
        assert!(!off_rails.rewritten && off_rails.fallback.unwrap().contains("did not look like a cleanup"));
        let down = compose(raw, &target, Category::Other, &settings, &store, Some(&llm)).unwrap();
        assert_eq!(
            down.fallback.as_deref(),
            Some("Could not reach 127.0.0.1:11434. Is it running?")
        );
        assert_eq!(
            compose(raw, &target, Category::Other, &settings, &store, Some(&llm)),
            None
        );
    }

    #[test]
    fn rewrite_failures_cool_down_then_probe_without_sleeping() {
        let store = Store::memory();
        store.add_word("sumit", "Sumedh", "manual", NOW).unwrap();
        let settings = Settings::default();
        let llm = Scripted::new(vec![
            Err("timeout".into()),
            Ok("1. Tell sumit about the launch tomorrow".into()),
            Err("timeout".into()),
            Ok("Tell sumit about the launch tomorrow".into()),
            Err("down".into()),
        ]);
        let target = Target::default();
        let mut circuit = RewriteCircuit::default();
        let mut run = |now| {
            compose_guarded(
                "tell sumit about the launch tomorrow",
                &target,
                Category::Work,
                &settings,
                &store,
                Some(&llm),
                (&mut circuit, now),
            )
            .unwrap()
        };
        assert!(run(0).fallback.unwrap().contains("timeout"));
        assert!(run(100).fallback.unwrap().contains("did not look"));
        let skipped = run(60_099);
        assert_eq!(skipped.text, "Tell Sumedh about the launch tomorrow.");
        assert_eq!(skipped.rewrite_ms, 0);
        assert!(skipped.fallback.unwrap().contains("temporarily skipped"));
        assert!(run(60_100).fallback.unwrap().contains("timeout"));
        assert!(run(120_100).rewritten);
        assert!(run(120_101).fallback.unwrap().contains("down"));
        assert!(!circuit.cooling(120_102));
        assert_eq!(llm.seen.borrow().len(), 5);
        assert_eq!(store.dictionary().unwrap()[0].hits, 6);
    }

    #[test]
    fn a_cooling_breaker_still_returns_nothing_for_fillers() {
        let store = Store::memory();
        let mut settings = Settings::default();
        settings.rewrite.min_words = 0;
        let llm = Scripted::new(vec![]);
        let mut circuit = RewriteCircuit {
            failures: 2,
            retry_at_ms: 100,
        };
        assert_eq!(
            compose_guarded(
                "um uh",
                &Target::default(),
                Category::Work,
                &settings,
                &store,
                Some(&llm),
                (&mut circuit, 0)
            ),
            None
        );
    }

    #[test]
    fn short_dictations_and_disabled_models_skip_the_rewrite() {
        let store = Store::memory();
        let mut settings = Settings::default();
        let llm = Scripted::new(vec![]);
        let target = Target::default();
        let short = compose(
            "sounds good to me",
            &target,
            Category::Personal,
            &settings,
            &store,
            Some(&llm),
        )
        .unwrap();
        assert_eq!(
            (short.text.as_str(), short.rewritten, short.fallback),
            ("Sounds good to me", false, None)
        );
        settings.rewrite.enabled = false;
        compose(
            "this one is long enough to be rewritten normally",
            &target,
            Category::Other,
            &settings,
            &store,
            Some(&llm),
        )
        .unwrap();
        assert!(llm.seen.borrow().is_empty());
        assert_eq!(compose("  ", &target, Category::Other, &settings, &store, None), None);
        assert_eq!(compose("um", &target, Category::Other, &settings, &store, None), None);
    }

    #[test]
    fn raw_mode_applies_only_the_users_own_words() {
        let store = Store::memory();
        store.save_snippet(None, "my email", "me@example.com").unwrap();
        let mut settings = Settings::default();
        settings.rewrite.level = Cleanup::None;
        let composed = compose(
            "um send it to my email",
            &Target::default(),
            Category::Other,
            &settings,
            &store,
            None,
        )
        .unwrap();
        assert_eq!(composed.text, "um send it to me@example.com");
        assert_eq!(composed.fixes, 1);
        assert_eq!(store.snippets().unwrap()[0].hits, 1);
    }

    #[test]
    fn press_enter_is_opt_in_and_only_at_the_end() {
        let store = Store::memory();
        let mut settings = Settings::default();
        settings.rewrite.enabled = false;
        settings.styles.other = Style::Formal;
        let target = Target::default();
        let off = compose(
            "search for flights, press enter.",
            &target,
            Category::Other,
            &settings,
            &store,
            None,
        )
        .unwrap();
        assert_eq!(
            (off.text.as_str(), off.press_enter),
            ("Search for flights, press enter.", false)
        );
        settings.press_enter = true;
        let on = compose(
            "search for flights, press enter.",
            &target,
            Category::Other,
            &settings,
            &store,
            None,
        )
        .unwrap();
        assert_eq!((on.text.as_str(), on.press_enter), ("Search for flights.", true));
        let alone = compose("Press Enter", &target, Category::Other, &settings, &store, None).unwrap();
        assert_eq!((alone.text.as_str(), alone.press_enter), ("", true));
        let middle = compose(
            "press enter to continue",
            &target,
            Category::Other,
            &settings,
            &store,
            None,
        )
        .unwrap();
        assert!(!middle.press_enter);
        let mut asr = Fixed::new(&["press enter"]);
        let audio = tone(1500, 0.3);
        let (paste, _, enter) = text(run(&request(&audio, "A"), &settings, &store, &mut asr, None));
        assert_eq!((paste.as_str(), enter), ("", true));
    }

    #[test]
    fn terminals_get_no_trailing_space_or_restyling() {
        let store = Store::memory();
        let mut settings = Settings::default();
        settings.rewrite.enabled = false;
        let mut asr = Fixed::new(&["git status"]);
        let audio = tone(1000, 0.3);
        let (paste, record, _) = text(run(
            &request(&audio, "Windows Terminal"),
            &settings,
            &store,
            &mut asr,
            None,
        ));
        assert_eq!(paste, "Git status");
        assert_eq!(record.category, Category::Terminal);
    }

    #[test]
    fn command_mode_rewrites_the_selection() {
        let store = Store::memory();
        let settings = Settings::default();
        let audio = tone(1500, 0.3);
        let mut req = request(&audio, "Notes");
        req.command = true;
        req.selection = "helo wrld";
        let mut asr = Fixed::new(&["fix the typos", "fix the typos", "fix the typos"]);
        let llm = Scripted::new(vec![Ok("\"Hello world\"".into()), Ok(" ".into())]);
        let (paste, record, _) = text(run(&req, &settings, &store, &mut asr, Some(&llm)));
        assert_eq!(paste, "Hello world");
        assert!(record.command && record.rewritten);
        assert_eq!(record.raw, "fix the typos");
        assert!(llm.seen.borrow()[0].1.contains("helo wrld"));
        assert_eq!(
            run(&req, &settings, &store, &mut asr, Some(&llm)).unwrap_err(),
            "The language model returned nothing for that command"
        );
        assert!(run(&req, &settings, &store, &mut asr, None)
            .unwrap_err()
            .starts_with("Command Mode needs the language model"));
    }
}
