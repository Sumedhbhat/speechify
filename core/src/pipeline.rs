//! One dictation from audio to text: trim, recognise, clean up, record.

use crate::asr::{Hint, Recognizer};
use crate::context::{categorize, Category, Target};
use crate::format::{self, Edits, Options};
use crate::live::Live;
use crate::llm::{cleanup_prompt, command_prompt, vet_cleanup, vet_command, Brief, Rewriter};
use crate::settings::{Cleanup, Settings, Style};
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
    circuit: (&mut RewriteCircuit, u64),
) -> Option<Composed> {
    let to = Destination {
        target,
        category,
        settings,
        store,
        rewriter,
    };
    compose_part(raw, &to, circuit, Part::Whole)
}

/// Where composed text is going and with what.
pub struct Destination<'a> {
    pub target: &'a Target,
    pub category: Category,
    pub settings: &'a Settings,
    pub store: &'a Store,
    pub rewriter: Option<&'a dyn Rewriter>,
}

/// Which part of a dictation is being composed.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Part<'a> {
    /// All of it at once.
    Whole,
    /// A block that `before` precedes. Blocks get formal punctuation here;
    /// the real style is applied once to the joined text, so that a style
    /// that drops a closing full stop does not drop one in the middle.
    /// Only the last block can end in "press enter".
    Block { before: &'a str, last: bool },
}

fn compose_part(
    raw: &str,
    to: &Destination,
    (circuit, now_ms): (&mut RewriteCircuit, u64),
    part: Part,
) -> Option<Composed> {
    let Destination {
        target,
        category,
        settings,
        store,
        rewriter,
    } = *to;
    let (before, last) = match part {
        Part::Whole => ("", true),
        Part::Block { before, last } => (before, last),
    };
    let mut out = Composed::default();
    let mut heard = raw.trim().to_string();
    if last && settings.press_enter && PRESS_ENTER.is_match(&heard) {
        heard = PRESS_ENTER.replace(&heard, "").trim().to_string();
        out.press_enter = true;
    }
    if heard.is_empty() {
        // "Press enter" alone still presses Enter.
        return out.press_enter.then_some(out);
    }
    let rules = store.replacement_rules();
    let style = category.style(&settings.styles);
    let options = Options {
        style: if part == Part::Whole { style } else { Style::Formal },
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
            style,
            category,
            spelling: settings.spelling,
            numbers: settings.numbers,
            target,
            vocabulary: &vocabulary,
            examples: &examples,
            before,
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

/// Cleans up the rest of a dictation after the blocks cleaned up while it
/// was spoken, and joins them, in the destination's style.
fn finish_blocks(
    blocks: Vec<Composed>,
    rest: &[String],
    to: &Destination,
    circuit: (&mut RewriteCircuit, u64),
) -> Option<Composed> {
    let before = join_blocks(blocks.iter().map(|block| block.text.as_str()));
    let last = compose_part(
        &rest.join(" "),
        to,
        circuit,
        Part::Block {
            before: &before,
            last: true,
        },
    );
    let mut out = last.clone().unwrap_or_default();
    let all: Vec<&Composed> = blocks.iter().chain(last.as_ref()).collect();
    let joined = join_blocks(all.iter().map(|block| block.text.as_str()));
    out.text = if to.category.literal() {
        joined
    } else {
        format::apply_style(&joined, to.category.style(&to.settings.styles))
    };
    out.rewritten = all.iter().any(|block| block.rewritten);
    out.fixes = all.iter().map(|block| block.fixes).sum();
    out.fallback = all.iter().rev().find_map(|block| block.fallback.clone());
    (!out.text.is_empty() || out.press_enter).then_some(out)
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
    run_live(
        request,
        Draft::default(),
        settings,
        store,
        recognizer,
        rewriter,
        circuit,
    )
}

/// Words a block must reach before it is cleaned up while the user talks.
/// Shorter blocks give the language model too little to work with.
pub const BLOCK_WORDS: usize = 40;

/// A dictation in progress: recognised piece by piece as it is spoken, and,
/// when the language model is on, cleaned up in blocks of whole sentences.
/// When the user stops, only the last block is left to clean up, so the wait
/// after a long dictation is no longer than after a short one.
#[derive(Debug, Default)]
pub struct Draft {
    pub live: Live,
    /// Cleaned-up blocks, in order.
    blocks: Vec<Composed>,
    /// How many of `live`'s pieces the blocks cover.
    used: usize,
    /// A correction may reach into a block already cleaned up, so the whole
    /// dictation is cleaned up at the end after all.
    whole: bool,
}

impl Draft {
    pub fn new(live: Live) -> Draft {
        Draft {
            live,
            ..Draft::default()
        }
    }

    /// The text cleaned up so far.
    fn written(&self) -> String {
        join_blocks(self.blocks.iter().map(|block| block.text.as_str()))
    }

    /// Cleans up the sentences recognised since the last block, if there are
    /// enough of them and they end a sentence. Only worth doing when the
    /// language model would be used: the rules alone take no time at the end.
    /// Returns whether a block was cleaned up.
    pub fn clean(&mut self, to: &Destination, circuit: (&mut RewriteCircuit, u64)) -> bool {
        let rewrite = &to.settings.rewrite;
        let modelled = rewrite.enabled && rewrite.level != Cleanup::None && to.rewriter.is_some();
        let pending = self.live.pieces()[self.used..].join(" ");
        let words = pending.split_whitespace().count();
        let finished = pending.trim_end().ends_with(['.', '?', '!']);
        // "Press enter" only counts at the very end, which is not known yet.
        let command = to.settings.press_enter && PRESS_ENTER.is_match(&pending);
        if self.used > 0 && format::reaches_back(&pending) {
            self.whole = true;
        }
        if !modelled || self.whole || circuit.0.cooling(circuit.1) || words < BLOCK_WORDS || !finished || command {
            return false;
        }
        let before = self.written();
        let composed = compose_part(
            &pending,
            to,
            circuit,
            Part::Block {
                before: &before,
                last: false,
            },
        );
        self.used = self.live.pieces().len();
        // Pure filler comes back as nothing, which leaves nothing to type.
        self.blocks.extend(composed);
        true
    }
}

/// Joins blocks of text: a numbered or bulleted line starts on a line of its
/// own, and so does whatever follows one.
fn join_blocks<'a>(blocks: impl Iterator<Item = &'a str>) -> String {
    static ITEM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d+\.|-) ").unwrap());
    let mut out = String::new();
    for block in blocks.filter(|block| !block.is_empty()) {
        if !out.is_empty() {
            let previous = out.lines().last().unwrap_or_default();
            out.push(if ITEM.is_match(block) || ITEM.is_match(previous) { '\n' } else { ' ' });
        }
        out.push_str(block);
    }
    out
}

/// Finishes a dictation whose start `draft` may already have recognised,
/// and partly cleaned up, while it was being recorded.
pub fn run_live(
    request: &Request,
    draft: Draft,
    settings: &Settings,
    store: &Store,
    recognizer: &mut dyn Recognizer,
    rewriter: Option<&dyn Rewriter>,
    circuit: (&mut RewriteCircuit, u64),
) -> Result<Outcome> {
    let started = Instant::now();
    let vocabulary = store.vocabulary();
    let hint = Hint {
        language: &settings.speech.language,
        vocabulary: &vocabulary,
        previous: "",
    };
    let Draft {
        live,
        blocks,
        used,
        whole,
    } = draft;
    let Some(heard) = live.finish(request.audio, recognizer, &hint)? else { return Ok(Outcome::Silent) };
    let raw = heard.raw;
    let asr_ms = started.elapsed().as_millis() as u32;
    if raw.is_empty() {
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
        speech_ms: heard.speech_ms as u32,
        raw: raw.clone(),
        heard: heard.heard,
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
        let to = Destination {
            target: &request.target,
            category,
            settings,
            store,
            rewriter,
        };
        let rest = &heard.parts[used.min(heard.parts.len())..];
        // Blocks that came to nothing still used their words. A correction
        // that may undo part of a cleaned-up block means starting over.
        let composed = if used == 0 || whole || format::reaches_back(&rest.join(" ")) {
            compose_part(&raw, &to, circuit, Part::Whole)
        } else {
            finish_blocks(blocks, rest, &to, circuit)
        };
        let Some(composed) = composed else {
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

    /// Forty-odd words, as a recogniser might hear the first part of a long dictation.
    const FIRST: &str = "um so the plan for the launch is simple we finish the review on monday we fix whatever it finds on tuesday and wednesday and we ship on thursday morning if nothing else comes up before then and the build stays green.";

    /// A draft whose first piece, `first`, was recognised while recording.
    fn drafted(asr: &mut Fixed, audio: &[f32]) -> Draft {
        let mut live = Live::default();
        let hint = Hint {
            language: "en",
            vocabulary: &[],
            previous: "",
        };
        assert!(live.advance(&audio[..5000 * 16], asr, &hint).unwrap());
        Draft::new(live)
    }

    fn spoken() -> Vec<f32> {
        [tone(4000, 0.3), vec![0.0; 16_000], tone(2000, 0.3)].concat()
    }

    #[test]
    fn a_long_dictation_is_cleaned_up_in_blocks_while_it_is_spoken() {
        let store = Store::memory();
        let settings = Settings {
            press_enter: true,
            ..Settings::default()
        };
        let audio = spoken();
        let mut asr = Fixed::detecting(&[FIRST, "and that is all for this week press enter"]);
        let mut draft = drafted(&mut asr, &audio);
        let llm = Scripted::new(vec![
            Ok("The plan for the launch is simple. We finish the review on Monday, fix what it finds on Tuesday and Wednesday, and ship on Thursday morning if the build stays green.".into()),
            Ok("And that is all for this week.".into()),
        ]);
        let target = Target {
            app: "WhatsApp".into(),
            ..Target::default()
        };
        let mut circuit = RewriteCircuit::default();
        let to = Destination {
            target: &target,
            category: Category::Personal,
            settings: &settings,
            store: &store,
            rewriter: Some(&llm),
        };
        // The first block is cleaned up before the user has stopped.
        assert!(draft.clean(&to, (&mut circuit, 0)));
        assert_eq!(llm.seen.borrow().len(), 1);
        assert!(!llm.seen.borrow()[0].1.contains("EARLIER"));
        // Nothing new: nothing more to clean up.
        assert!(!draft.clean(&to, (&mut circuit, 0)));
        let outcome = run_live(
            &request(&audio, "WhatsApp"),
            draft,
            &settings,
            &store,
            &mut asr,
            Some(&llm),
            (&mut circuit, 0),
        );
        let (paste, record, enter) = text(outcome);
        // Casual style drops only the closing full stop, not the one between blocks.
        assert_eq!(
            paste,
            "The plan for the launch is simple. We finish the review on Monday, fix what it finds on Tuesday and Wednesday, and ship on Thursday morning if the build stays green. And that is all for this week "
        );
        assert!(enter && record.rewritten);
        assert_eq!(record.raw, format!("{FIRST} and that is all for this week press enter"));
        // The last block was told what came before it.
        let seen = llm.seen.borrow();
        assert!(seen[1].1.contains("EARLIER (already typed") && seen[1].1.contains("build stays green."));
        assert!(seen[1].1.contains("RAW:\n<<<\nand that is all for this week\n>>>"));
    }

    #[test]
    fn blocks_wait_for_enough_finished_sentences_and_a_working_model() {
        let store = Store::memory();
        let settings = Settings::default();
        let target = Target::default();
        let llm = Scripted::new(vec![]);
        let mut circuit = RewriteCircuit::default();
        let to = |rewriter| Destination {
            target: &target,
            category: Category::Other,
            settings: &settings,
            store: &store,
            rewriter,
        };
        let audio = spoken();
        for first in ["too short to bother with.", &FIRST.replace('.', "")] {
            let mut asr = Fixed::detecting(&[first]);
            let mut draft = drafted(&mut asr, &audio);
            assert!(!draft.clean(&to(Some(&llm)), (&mut circuit, 0)), "{first}");
        }
        let mut asr = Fixed::detecting(&[FIRST]);
        let mut draft = drafted(&mut asr, &audio);
        assert!(!draft.clean(&to(None), (&mut circuit, 0)));
        let mut cooling = RewriteCircuit {
            failures: 2,
            retry_at_ms: 100,
        };
        assert!(!draft.clean(&to(Some(&llm)), (&mut cooling, 0)));
        let mut off = settings.clone();
        off.rewrite.level = Cleanup::None;
        let raw = Destination {
            settings: &off,
            ..to(Some(&llm))
        };
        assert!(!draft.clean(&raw, (&mut circuit, 0)));
        assert!(llm.seen.borrow().is_empty());
    }

    #[test]
    fn press_enter_and_corrections_keep_their_meaning_across_blocks() {
        let store = Store::memory();
        let settings = Settings {
            press_enter: true,
            ..Settings::default()
        };
        let target = Target::default();
        let audio = spoken();
        let llm = Scripted::new(vec![Ok("All of it, cleaned at the end.".into())]);
        let to = Destination {
            target: &target,
            category: Category::Other,
            settings: &settings,
            store: &store,
            rewriter: Some(&llm),
        };
        let mut circuit = RewriteCircuit::default();
        // A block that ends in "press enter" may be the end: it waits.
        let ending = format!("{} press enter.", FIRST.trim_end_matches('.'));
        let mut asr = Fixed::detecting(&[&ending]);
        let mut draft = drafted(&mut asr, &audio);
        assert!(!draft.clean(&to, (&mut circuit, 0)));
        let (paste, _, enter) = text(run_live(
            &request(&audio[..5000 * 16], "A"),
            draft,
            &settings,
            &store,
            &mut asr,
            Some(&llm),
            (&mut circuit, 0),
        ));
        assert!(enter && !paste.to_lowercase().contains("press enter"), "{paste}");

        // "Scratch that" after a cleaned-up block: the whole is cleaned up again.
        let llm = Scripted::new(vec![
            Ok("The plan is to ship on Thursday.".into()),
            Ok("The plan is to ship on Friday.".into()),
        ]);
        let to = Destination {
            rewriter: Some(&llm),
            ..to
        };
        let mut asr = Fixed::detecting(&[FIRST, "scratch that we ship on friday"]);
        let mut draft = drafted(&mut asr, &audio);
        assert!(draft.clean(&to, (&mut circuit, 0)));
        let (paste, _, _) = text(run_live(
            &request(&audio, "A"),
            draft,
            &settings,
            &store,
            &mut asr,
            Some(&llm),
            (&mut circuit, 0),
        ));
        assert_eq!(paste, "The plan is to ship on Friday. ");
        assert!(llm.seen.borrow()[1].1.contains(FIRST) && !llm.seen.borrow()[1].1.contains("EARLIER"));

        // A correction in a later piece stops cleaning up in blocks altogether.
        let llm = Scripted::new(vec![
            Ok("First.".into()),
            Ok("The plan for the launch is simple.".into()),
        ]);
        let to = Destination {
            rewriter: Some(&llm),
            ..to
        };
        let correction = format!("no wait {}", FIRST.replace("monday", "tuesday"));
        let longer = [spoken(), vec![0.0; 16_000]].concat();
        let mut asr = Fixed::detecting(&[FIRST, &correction]);
        let mut draft = drafted(&mut asr, &longer);
        assert!(draft.clean(&to, (&mut circuit, 0)));
        let hint = Hint::default();
        let now = &longer[draft.live.done()..];
        assert!(draft.live.advance(now, &mut asr, &hint).unwrap());
        assert!(!draft.clean(&to, (&mut circuit, 0)));
        assert!(draft.whole);
        let (paste, _, _) = text(run_live(
            &request(&longer, "A"),
            draft,
            &settings,
            &store,
            &mut asr,
            Some(&llm),
            (&mut circuit, 0),
        ));
        assert_eq!(paste, "The plan for the launch is simple. ");
    }

    #[test]
    fn blocks_join_lists_on_their_own_lines_and_skip_what_came_to_nothing() {
        assert_eq!(
            join_blocks(["We need:\n1. A budget", "2. A colour scheme", "", "Thanks."].into_iter()),
            "We need:\n1. A budget\n2. A colour scheme\nThanks."
        );
        assert_eq!(join_blocks(["One.", "Two."].into_iter()), "One. Two.");
        let store = Store::memory();
        let settings = Settings::default();
        let target = Target::default();
        let audio = spoken();
        // A block of filler, then a terminal: no style, nothing from the filler.
        let filler = format!("{}um.", "um uh ".repeat(20));
        let mut asr = Fixed::detecting(&[&filler, "git status"]);
        let mut draft = drafted(&mut asr, &audio);
        let llm = Scripted::new(vec![Ok("EMPTY".into())]);
        let mut circuit = RewriteCircuit::default();
        let to = Destination {
            target: &target,
            category: Category::Terminal,
            settings: &settings,
            store: &store,
            rewriter: Some(&llm),
        };
        assert!(draft.clean(&to, (&mut circuit, 0)));
        let (paste, record, _) = text(run_live(
            &request(&audio, "Windows Terminal"),
            draft,
            &settings,
            &store,
            &mut asr,
            Some(&llm),
            (&mut circuit, 0),
        ));
        assert_eq!(paste, "git status");
        assert!(!record.rewritten);
        // Everything came to nothing: nothing is typed.
        let mut asr = Fixed::detecting(&[&filler, "um"]);
        let mut draft = drafted(&mut asr, &audio);
        let llm = Scripted::new(vec![Ok("EMPTY".into())]);
        let to = Destination {
            rewriter: Some(&llm),
            category: Category::Other,
            ..to
        };
        assert!(draft.clean(&to, (&mut circuit, 0)));
        let outcome = run_live(
            &request(&audio, "A"),
            draft,
            &settings,
            &store,
            &mut asr,
            Some(&llm),
            (&mut circuit, 0),
        );
        assert_eq!(outcome.unwrap(), Outcome::Nothing);
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
            ..Fixed::new(&[])
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
        let empty = compose(raw, &target, Category::Other, &settings, &store, Some(&llm)).unwrap();
        assert_eq!(empty.text, "What is the capital of france please tell me.");
        assert!(!empty.rewritten && empty.fallback.is_some());
        let filler = Scripted::new(vec![Ok("EMPTY".into())]);
        let mut settings = settings;
        settings.rewrite.min_words = 0;
        assert_eq!(
            compose("uh um", &target, Category::Other, &settings, &store, Some(&filler)),
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
        assert_eq!(paste, "git status");
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
