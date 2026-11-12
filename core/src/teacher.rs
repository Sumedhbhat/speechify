//! The nightly teacher: a larger speech model and a larger language model
//! review the day's dictations, and what they find teaches the everyday
//! (student) models.
//!
//! Nothing is fine-tuned on the user's machine. The student improves in
//! three ways: recurring recognition mistakes become dictionary corrections,
//! ideal results become worked examples in the cleanup prompt, and every
//! reviewed dictation can be exported as training data.

use crate::asr::{strip_tags, Hint, Recognizer};
use crate::context::Target;
use crate::diff::{normalize, wer, words};
use crate::format::Replacement;
use crate::learn::{corrections, corroborated, evidence, notable_terms, Evidence};
use crate::llm::{vet_cleanup, Rewriter};
use crate::pipeline::compose;
use crate::settings::Settings;
use crate::store::{Clock, Dictation, Store, MAX_TERM};
use crate::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Report {
    pub reviewed: u32,
    /// Recordings that could not be read.
    pub skipped: u32,
    /// The everyday model's word error rate against the teacher.
    pub average_wer: Option<f64>,
    /// Corrections added to the dictionary in this run.
    pub learned: Vec<Replacement>,
    /// Corrections seen but not yet often enough to add.
    pub pending: u32,
    /// Worked examples added to the cleanup prompt.
    pub examples: u32,
}

fn same_words(a: &str, b: &str) -> bool {
    let fold = |text: &str| words(text).into_iter().map(normalize).collect::<Vec<_>>();
    fold(a) == fold(b)
}

/// Counts corrections and adds each to the dictionary once it has been seen
/// `learn_after` times.
struct Learner<'a> {
    store: &'a Store,
    settings: &'a Settings,
    clock: Clock,
    student: &'a str,
    /// The recogniser's confidence in each word of `student`.
    heard: &'a [crate::asr::Heard],
}

impl Learner<'_> {
    /// Counts corrections by the evidence for each. A correction that changes
    /// the meaning is dropped. One that sounds like what was heard is learned
    /// at once if the recogniser was unsure of those words, and otherwise only
    /// once it has come up `learn_after` times.
    fn take(&self, found: Vec<Replacement>, source: &str, report: &mut Report) -> Result<()> {
        for correction in found {
            if correction.from.chars().count() > MAX_TERM || correction.to.chars().count() > MAX_TERM {
                continue;
            }
            // Zero means it is already in the dictionary, or the user deleted it.
            let enough = self.settings.teacher.learn_after;
            let weight = match evidence(&correction, self.heard) {
                Evidence::None => continue,
                Evidence::Strong => enough,
                Evidence::Weak
                    if source == "teacher" && corroborated(&correction, self.student, &self.store.vocabulary()) =>
                {
                    enough
                }
                Evidence::Weak => 1,
            };
            let count = self.store.suggest(&correction.from, &correction.to, weight)?;
            if count >= self.settings.teacher.learn_after && self.settings.auto_learn {
                self.store
                    .accept(&correction.from, &correction.to, source, self.clock)?;
                report.learned.push(correction);
            } else if count > 0 {
                report.pending += 1;
            }
        }
        Ok(())
    }
}

/// Records the user's corrected text for a dictation and learns from it at
/// once: a correction made by hand needs no second opinion.
pub fn learn_from_edit(
    store: &Store,
    settings: &Settings,
    id: i64,
    text: &str,
    clock: Clock,
) -> Result<Vec<Replacement>> {
    let before = store.dictation(id)?;
    store.set_edited(id, text)?;
    let mut report = Report::default();
    if settings.auto_learn && !before.command {
        let learner = Learner {
            store,
            settings,
            clock,
            student: &before.raw,
            heard: &before.heard,
        };
        learner.take(corrections(&before.text, text), "edit", &mut report)?;
    }
    if !same_words(&before.text, text) && !text.trim().is_empty() && !before.command {
        store.add_example(before.category, &before.raw, text.trim(), clock)?;
    } else if before.edited.is_some()
        && !before.command
        && store.remove_example(&before.raw, before.edited.as_deref())?
    {
        // Another dictation of the same words may still have a valid user edit.
        if let Some(other) = store.all()?.into_iter().rev().find(|d| {
            d.id != id
                && !d.command
                && d.raw == before.raw
                && d.edited.as_ref().is_some_and(|gold| !same_words(gold, &d.text))
        }) {
            store.add_example(other.category, &other.raw, other.edited.as_ref().unwrap(), clock)?;
        }
    }
    Ok(report.learned)
}

/// Reviews unreviewed dictations. `load_audio` reads a stored recording;
/// `keep_going` is polled between dictations so the run can stop when the
/// user starts dictating.
pub fn review(
    store: &Store,
    settings: &Settings,
    recognizer: &mut dyn Recognizer,
    rewriter: Option<&dyn Rewriter>,
    load_audio: &dyn Fn(&str) -> Result<Vec<f32>>,
    clock: Clock,
    keep_going: &dyn Fn() -> bool,
) -> Result<Report> {
    let mut report = Report::default();
    let mut total_wer = 0.0;
    let vocabulary = store.vocabulary();
    for dictation in store.pending_review(settings.teacher.batch)? {
        if !keep_going() {
            break;
        }
        let Ok(samples) = load_audio(&dictation.audio) else {
            store.skip_review(dictation.id, clock)?;
            report.skipped += 1;
            continue;
        };
        let trimmed = crate::asr::isolate(recognizer, &samples)?;
        if trimmed.samples.is_empty() {
            store.skip_review(dictation.id, clock)?;
            report.skipped += 1;
            continue;
        }
        let heard = strip_tags(&recognizer.transcribe(
            &trimmed.samples,
            &Hint {
                language: &settings.speech.language,
                vocabulary: &vocabulary,
            },
        )?);
        let score = wer(&heard, &dictation.raw);
        let target = Target {
            app: dictation.app.clone(),
            process: dictation.process.clone(),
            title: dictation.title.clone(),
        };
        // Commands are instructions, not text: only their recognition is reviewed.
        let ideal = if dictation.command {
            String::new()
        } else {
            // The teacher always rewrites, however short the dictation.
            let mut strict = settings.clone();
            strict.rewrite.min_words = 0;
            strict.rewrite.enabled = true;
            strict.press_enter = false;
            compose(&heard, &target, dictation.category, &strict, store, rewriter)
                .map(|c| c.text)
                .unwrap_or_default()
        };
        store.save_review(dictation.id, &heard, &ideal, score, clock)?;
        report.reviewed += 1;
        total_wer += score;
        let agreed = wer(&heard, &dictation.raw).max(wer(&dictation.raw, &heard)) <= 0.4;
        if agreed {
            let learner = Learner {
                store,
                settings,
                clock,
                student: &dictation.raw,
                heard: &dictation.heard,
            };
            learner.take(corrections(&dictation.raw, &heard), "teacher", &mut report)?;
            if settings.auto_learn {
                // Shared recognition is independent evidence of the spelling.
                // Vocabulary biases capable recognisers but never fuzzy-rewrites
                // ordinary words in the formatter.
                for term in notable_terms(&heard) {
                    if words(&dictation.raw)
                        .iter()
                        .any(|word| normalize(word) == normalize(&term))
                    {
                        store.add_word("", &term, "teacher", clock)?;
                    }
                }
            }
        }
        // What the user typed over it outranks the teacher's opinion.
        let trusted = dictation.edited.is_some();
        let safe = agreed && plausible_example(&heard, &ideal) && plausible_example(&dictation.raw, &ideal);
        let gold = dictation.edited.clone().unwrap_or(ideal);
        if !dictation.command && (trusted || safe) && !gold.is_empty() && !same_words(&gold, &dictation.text) {
            store.add_example(dictation.category, &dictation.raw, &gold, clock)?;
            report.examples += 1;
        }
    }
    report.average_wer = (report.reviewed > 0).then(|| total_wer / report.reviewed as f64);
    Ok(report)
}

fn plausible_example(raw: &str, ideal: &str) -> bool {
    let (before, after) = (words(raw).len(), words(ideal).len());
    let spoken: Vec<String> = words(raw).into_iter().map(normalize).collect();
    let kept = words(ideal)
        .iter()
        .filter(|word| spoken.contains(&normalize(word)))
        .count();
    after * 2 >= before && after <= before * 3 / 2 + 2 && kept * 2 >= after && vet_cleanup(raw, ideal).is_some()
}

/// Whether tonight's review should start: after the set hour, once a day.
pub fn due(settings: &Settings, last_run: Option<i64>, clock: Clock) -> bool {
    let local = |time: i64| time + clock.offset_min as i64 * 60;
    let today = local(clock.now).div_euclid(86_400);
    let hour = local(clock.now).rem_euclid(86_400) / 3600;
    settings.teacher.enabled
        && hour >= settings.teacher.hour as i64
        && last_run.is_none_or(|run| local(run).div_euclid(86_400) < today)
}

/// Reviewed dictations as JSON Lines, for fine-tuning a speech or cleanup
/// model elsewhere: the recording, what each model heard and the ideal text.
pub fn export(dictations: &[Dictation]) -> String {
    let mut out = String::new();
    for d in dictations
        .iter()
        .filter(|d| d.teacher_raw.is_some() && !d.audio.is_empty())
    {
        let line = serde_json::json!({
            "audio": d.audio,
            "heard": d.raw,
            "teacher_heard": d.teacher_raw,
            "typed": d.text,
            "ideal": if d.command { None } else { d.edited.as_ref().or(d.teacher_text.as_ref()) },
            "command": d.command,
            "corrected_by_user": d.edited.is_some(),
            "app": d.app,
            "category": d.category,
        });
        out.push_str(&line.to_string());
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asr::Fixed;
    use crate::audio::tone;
    use crate::context::Category;
    use crate::llm::Scripted;
    use crate::testing::dictation;

    const NOW: Clock = Clock {
        now: 1_800_000_000,
        offset_min: 330,
    };

    fn recorded(store: &Store, raw: &str, text: &str, audio: &str) -> i64 {
        store
            .add_dictation(&Dictation {
                raw: raw.into(),
                audio: audio.into(),
                category: Category::Work,
                ..dictation(text, "Slack", NOW.now - 100)
            })
            .unwrap()
    }

    fn loader(name: &str) -> Result<Vec<f32>> {
        if name == "missing.wav" {
            Err("gone".into())
        } else {
            Ok(tone(1000, 0.3))
        }
    }

    #[test]
    fn recurring_mistakes_become_dictionary_corrections() {
        let store = Store::memory();
        let settings = Settings::default();
        let first = recorded(
            &store,
            "ask sumit bat about the launch plan",
            "Ask sumit bat about the launch plan.",
            "a.wav",
        );
        recorded(
            &store,
            "tell sumit bat that it shipped today",
            "Tell sumit bat that it shipped today.",
            "b.wav",
        );
        recorded(&store, "lost", "Lost.", "missing.wav");
        let mut teacher = Fixed::new(&[
            "Ask Sumedh Bhat about the launch plan.",
            "Tell Sumedh Bhat that it shipped today.",
        ]);
        let report = review(&store, &settings, &mut teacher, None, &loader, NOW, &|| true).unwrap();
        assert_eq!(
            (report.reviewed, report.skipped, report.pending, report.examples),
            (2, 1, 1, 2)
        );
        assert_eq!(report.learned, vec![Replacement::new("sumit bat", "Sumedh Bhat")]);
        assert!((report.average_wer.unwrap() - 2.0 / 7.0).abs() < 1e-9);
        let word = &store.dictionary().unwrap()[0];
        assert_eq!(
            (word.heard.as_str(), word.term.as_str(), word.source.as_str()),
            ("sumit bat", "Sumedh Bhat", "teacher")
        );
        let reviewed = store.dictation(first).unwrap();
        assert_eq!(
            reviewed.teacher_text.as_deref(),
            Some("Ask Sumedh Bhat about the launch plan.")
        );
        assert_eq!(reviewed.teacher_wer, Some(2.0 / 7.0));
        // The ideal texts became worked examples for the everyday model.
        assert_eq!(store.examples(Category::Work, 5).len(), 2);
        // Everything is reviewed now; a second run does nothing.
        let again = review(&store, &settings, &mut Fixed::new(&[]), None, &loader, NOW, &|| true).unwrap();
        assert_eq!(again, Report::default());
    }

    #[test]
    fn the_teacher_model_writes_the_ideal_text_and_user_edits_outrank_it() {
        let store = Store::memory();
        let settings = Settings {
            auto_learn: false,
            ..Settings::default()
        };
        let plain = recorded(&store, "um lets meet friday", "Lets meet friday.", "a.wav");
        let edited = recorded(&store, "call asha", "Call asha.", "b.wav");
        store.set_edited(edited, "Call Aasha now.").unwrap();
        let command = store
            .add_dictation(&Dictation {
                command: true,
                audio: "c.wav".into(),
                ..dictation("Shorter text", "Notes", NOW.now)
            })
            .unwrap();
        let mut teacher = Fixed::new(&["Um, let's meet Friday.", "Call Asha.", "make it shorter"]);
        let llm = Scripted::new(vec![Ok("Let's meet Friday.".into()), Ok("Call Asha.".into())]);
        let report = review(&store, &settings, &mut teacher, Some(&llm), &loader, NOW, &|| true).unwrap();
        assert_eq!((report.reviewed, report.examples), (3, 2));
        assert!(report.learned.is_empty());
        assert_eq!(
            store.dictation(plain).unwrap().teacher_text.as_deref(),
            Some("Let's meet Friday.")
        );
        assert_eq!(store.dictation(command).unwrap().teacher_text.as_deref(), Some(""));
        let examples = store.examples(Category::Work, 5);
        assert!(examples.contains(&("call asha".into(), "Call Aasha now.".into())));
        assert!(examples.contains(&("um lets meet friday".into(), "Let's meet Friday.".into())));
        // With learning off, the correction waits as a suggestion.
        assert!(store.dictionary().unwrap().is_empty());
        assert!(!store.suggestions().unwrap().is_empty());
    }

    #[test]
    fn unrelated_transcripts_and_invented_ideals_never_teach_examples() {
        let store = Store::memory();
        recorded(&store, "They're the same.", "They're the same.", "a.wav");
        recorded(
            &store,
            "I stay in a warm place so during summer uh",
            "I stay in a warm place so during summer.",
            "b.wav",
        );
        recorded(
            &store,
            "Please call Priya about the report tomorrow",
            "Please call Priya about the report tomorrow.",
            "c.wav",
        );
        let edited = recorded(&store, "They're the same.", "They're the same.", "d.wav");
        store
            .set_edited(edited, "My own unrelated replacement is trusted.")
            .unwrap();
        let mut asr = Fixed::new(&[
            "Participant Bata Redo and Podd contenido is an extreme I want to miss that",
            "I stay in a warm place so during summer uh",
            "Please call Priya about the report tomorrow",
            "A completely different sentence with many invented words",
        ]);
        let llm = Scripted::new(vec![
            Ok("Participant Bata Redo and Podd contenido is an extreme I want to miss that".into()),
            Ok("I stay in a warm place so during summer uh: 1. Warm Place.".into()),
            Ok("Priya.".into()),
            Ok("A completely different sentence with many invented words".into()),
        ]);
        let report = review(
            &store,
            &Settings::default(),
            &mut asr,
            Some(&llm),
            &loader,
            NOW,
            &|| true,
        )
        .unwrap();
        assert_eq!(report.examples, 1);
        assert_eq!(
            store.examples(Category::Work, 10)[0].1,
            "My own unrelated replacement is trusted."
        );
        assert!(!store.vocabulary().iter().any(|term| term == "Bata"));
        assert!(store.suggestions().unwrap().is_empty());
    }

    #[test]
    fn short_unrelated_ideals_are_not_worked_examples() {
        let store = Store::memory();
        recorded(&store, "bring files tomorrow", "Bring files tomorrow.", "a.wav");
        let llm = Scripted::new(vec![Ok("Delicious food awaits.".into())]);
        let report = review(
            &store,
            &Settings::default(),
            &mut Fixed::new(&["bring files tomorrow"]),
            Some(&llm),
            &loader,
            NOW,
            &|| true,
        )
        .unwrap();
        assert_eq!(report.examples, 0);
        assert!(store.examples(Category::Work, 1).is_empty());
    }

    #[test]
    fn legitimate_spoken_enumerations_can_teach_list_examples() {
        let store = Store::memory();
        let raw = "First, check the invoice. Second, pay the supplier.";
        recorded(&store, raw, raw, "a.wav");
        let mut asr = Fixed::new(&[raw]);
        let llm = Scripted::new(vec![Ok("1. Check the invoice\n2. Pay the supplier".into())]);
        let report = review(
            &store,
            &Settings::default(),
            &mut asr,
            Some(&llm),
            &loader,
            NOW,
            &|| true,
        )
        .unwrap();
        assert_eq!(report.examples, 1);
        assert_eq!(
            store.examples(Category::Work, 1)[0].1,
            "1. Check the invoice\n2. Pay the supplier"
        );
    }

    #[test]
    fn shared_names_supply_vocabulary_and_corroborate_a_first_mishearing() {
        let store = Store::memory();
        recorded(
            &store,
            "Please ask Kher about the report",
            "Please ask Kher about the report.",
            "a.wav",
        );
        recorded(
            &store,
            "Please ask Kerr about the report",
            "Please ask Kerr about the report.",
            "b.wav",
        );
        recorded(
            &store,
            "Please check the shorts before delivery",
            "Please check the shorts before delivery.",
            "c.wav",
        );
        let mut asr = Fixed::new(&[
            "Please ask Kher about the report",
            "Please ask Kher about the report",
            "Please check the Shorts before delivery",
        ]);
        let report = review(&store, &Settings::default(), &mut asr, None, &loader, NOW, &|| true).unwrap();
        assert_eq!(report.learned, vec![Replacement::new("kerr", "Kher")]);
        assert!(store.vocabulary().contains(&"Kher".into()));
        assert_eq!(
            store
                .dictionary()
                .unwrap()
                .iter()
                .filter(|word| word.heard == "kerr")
                .count(),
            1
        );
        let ordinary = "we wear shorts in may and call the next line of products new";
        let s = Settings::default();
        let compose = |store: &Store| {
            crate::pipeline::compose(ordinary, &Target::default(), Category::Work, &s, store, None)
                .unwrap()
                .text
        };
        assert_eq!(compose(&store).to_lowercase(), compose(&Store::memory()).to_lowercase());
        // Forgetting survives review; a plain vocabulary term cannot re-add itself.
        let word = store
            .dictionary()
            .unwrap()
            .into_iter()
            .find(|word| word.heard.is_empty() && word.term == "Kher")
            .unwrap();
        store.remove_word(word.id).unwrap();
        recorded(
            &store,
            "Please ask Kher about the report",
            "Please ask Kher about the report.",
            "d.wav",
        );
        review(
            &store,
            &s,
            &mut Fixed::new(&["Please ask Kher about the report"]),
            None,
            &loader,
            NOW,
            &|| true,
        )
        .unwrap();
        assert!(!store
            .dictionary()
            .unwrap()
            .iter()
            .any(|word| word.heard.is_empty() && word.term == "Kher"));
    }

    #[test]
    fn silent_history_is_skipped_without_calling_the_teacher() {
        let store = Store::memory();
        recorded(&store, "old words", "Old words.", "silent.wav");
        let mut asr = Fixed::new(&[]);
        let report = review(
            &store,
            &Settings::default(),
            &mut asr,
            None,
            &|_| Ok(vec![0.0; 16000]),
            NOW,
            &|| true,
        )
        .unwrap();
        assert_eq!((report.skipped, report.reviewed), (1, 0));
        assert!(asr.heard.is_empty());
        assert!(store.pending_review(1).unwrap().is_empty());
    }

    #[test]
    fn stops_when_asked_and_reports_engine_failures() {
        let store = Store::memory();
        recorded(&store, "one", "One.", "a.wav");
        let settings = Settings::default();
        let stopped = review(&store, &settings, &mut Fixed::new(&[]), None, &loader, NOW, &|| false).unwrap();
        assert_eq!(stopped.reviewed, 0);
        assert_eq!(stopped.average_wer, None);
        let mut broken = Fixed {
            answers: vec![Err("out of memory".into())],
            heard: vec![],
        };
        assert_eq!(
            review(&store, &settings, &mut broken, None, &loader, NOW, &|| true).unwrap_err(),
            "out of memory"
        );
    }

    /// A dictation as `recorded` makes it, with the recogniser's confidence in each raw word.
    fn heard_with(store: &Store, raw: &str, text: &str, sure: &[(&str, f32)]) -> i64 {
        let heard = raw
            .split_whitespace()
            .map(|word| crate::asr::Heard {
                word: word.into(),
                p: sure.iter().find(|(w, _)| *w == word).map_or(0.98, |(_, p)| *p),
            })
            .collect();
        store
            .add_dictation(&Dictation {
                raw: raw.into(),
                heard,
                category: Category::Work,
                ..dictation(text, "Slack", NOW.now)
            })
            .unwrap()
    }

    #[test]
    fn a_hand_fix_of_words_the_recogniser_was_unsure_of_is_learned_at_once() {
        let store = Store::memory();
        let settings = Settings::default();
        let raw = "deploy with cube control tonight please";
        let id = heard_with(
            &store,
            raw,
            "Deploy with cube control tonight please.",
            &[("cube", 0.41), ("control", 0.55)],
        );
        let learned = learn_from_edit(&store, &settings, id, "Deploy with kubectl tonight please.", NOW).unwrap();
        assert_eq!(learned, vec![Replacement::new("cube control", "kubectl")]);
        assert_eq!(store.dictionary().unwrap()[0].source, "edit");
        assert_eq!(
            store.examples(Category::Work, 1)[0].1,
            "Deploy with kubectl tonight please."
        );
    }

    #[test]
    fn a_hand_fix_of_confident_words_must_come_up_twice() {
        let store = Store::memory();
        let settings = Settings::default();
        let first = heard_with(&store, "ask sumit bat today", "Ask sumit bat today.", &[]);
        assert!(learn_from_edit(&store, &settings, first, "Ask Sumedh Bhat today.", NOW)
            .unwrap()
            .is_empty());
        assert_eq!(store.suggestions().unwrap()[0].count, 1);
        assert!(store.dictionary().unwrap().is_empty());
        // The same fix again, on a recording with no confidence at all, is enough.
        let second = recorded(
            &store,
            "tell sumit bat it shipped",
            "Tell sumit bat it shipped.",
            "b.wav",
        );
        let learned = learn_from_edit(&store, &settings, second, "Tell Sumedh Bhat it shipped.", NOW).unwrap();
        assert_eq!(learned, vec![Replacement::new("sumit bat", "Sumedh Bhat")]);
    }

    #[test]
    fn an_edit_that_changes_the_meaning_is_never_learned() {
        let store = Store::memory();
        let settings = Settings::default();
        // Even where the recogniser was unsure, "open" to "close" is a new
        // instruction, not a misheard word; nor is a change of number.
        let unsure = [("open", 0.3), ("three", 0.3)];
        for (raw, text, edit) in [
            ("open the file now", "Open the file now.", "Close the file now."),
            ("open the file now", "Open the file now.", "Close the file now."),
            ("order three boxes", "Order three boxes.", "Order two boxes."),
        ] {
            let id = heard_with(&store, raw, text, &unsure);
            assert!(learn_from_edit(&store, &settings, id, edit, NOW).unwrap().is_empty());
        }
        assert!(store.dictionary().unwrap().is_empty() && store.suggestions().unwrap().is_empty());
        // The corrected text is still a good example of what the user meant.
        assert_eq!(store.examples(Category::Work, 9).len(), 2);
    }

    #[test]
    fn edits_teach_nothing_when_learning_is_off_or_the_text_is_unchanged() {
        let store = Store::memory();
        let settings = Settings {
            auto_learn: false,
            ..Settings::default()
        };
        let other = heard_with(&store, "ping vikram soon", "Ping vikram soon.", &[("vikram", 0.2)]);
        assert!(learn_from_edit(&store, &settings, other, "Ping Vikraam soon.", NOW)
            .unwrap()
            .is_empty());
        assert!(store.dictionary().unwrap().is_empty());
        let third = recorded(&store, "all good", "All good.", "c.wav");
        learn_from_edit(&store, &settings, third, "All good!", NOW).unwrap();
        assert_eq!(store.examples(Category::Work, 9).len(), 1);
        assert!(learn_from_edit(&store, &settings, 999, "x", NOW).is_err());
    }

    #[test]
    fn runs_once_a_night_after_the_set_hour() {
        let settings = Settings::default();
        let midnight_ist = 20_467 * 86_400 - 330 * 60;
        let at = |hour: i64| Clock {
            now: midnight_ist + hour * 3600,
            offset_min: 330,
        };
        assert!(!due(&settings, None, at(1)));
        assert!(due(&settings, None, at(2)));
        assert!(due(&settings, Some(at(3).now - 86_400), at(3)));
        assert!(!due(&settings, Some(at(2).now), at(5)));
        let mut off = settings.clone();
        off.teacher.enabled = false;
        assert!(!due(&off, None, at(3)));
    }

    #[test]
    fn exports_reviewed_dictations_as_json_lines() {
        let reviewed = Dictation {
            audio: "a.wav".into(),
            teacher_raw: Some("Hello Asha".into()),
            teacher_text: Some("Hello Asha.".into()),
            ..dictation("Hello asha", "Slack", 1)
        };
        let edited = Dictation {
            edited: Some("Hello Aasha.".into()),
            ..reviewed.clone()
        };
        let unreviewed = Dictation {
            audio: "b.wav".into(),
            ..dictation("x", "A", 2)
        };
        let no_audio = Dictation {
            audio: String::new(),
            ..reviewed.clone()
        };
        let out = export(&[reviewed, edited, unreviewed, no_audio]);
        let lines: Vec<serde_json::Value> = out.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["ideal"], "Hello Asha.");
        assert_eq!(lines[0]["corrected_by_user"], false);
        assert_eq!(lines[1]["ideal"], "Hello Aasha.");
        assert_eq!(lines[1]["heard"], "hello asha");
        assert_eq!(lines[1]["category"], "other");
    }

    #[test]
    fn command_output_edits_never_teach_recognition_or_cleanup() {
        let store = Store::memory();
        let id = store
            .add_dictation(&Dictation {
                raw: "make it shorter".into(),
                command: true,
                audio: "a.wav".into(),
                ..dictation("Deploy kubectl tonight please", "Notes", NOW.now)
            })
            .unwrap();
        assert!(
            learn_from_edit(&store, &Settings::default(), id, "Deploy Docker tonight please", NOW)
                .unwrap()
                .is_empty()
        );
        assert!(store.dictionary().unwrap().is_empty());
        assert!(store.suggestions().unwrap().is_empty());
        assert!(store.examples(Category::Other, 5).is_empty());
        let report = review(
            &store,
            &Settings::default(),
            &mut Fixed::new(&["make it shorter"]),
            None,
            &loader,
            NOW,
            &|| true,
        )
        .unwrap();
        assert_eq!(report.examples, 0);
        assert!(store.examples(Category::Other, 5).is_empty());
        let exported: serde_json::Value = serde_json::from_str(export(&store.all().unwrap()).trim()).unwrap();
        assert_eq!(exported["command"], true);
        assert!(exported["ideal"].is_null());
        assert_eq!(exported["teacher_heard"], "make it shorter");
    }

    #[test]
    fn clearing_an_edit_removes_its_example_and_preserves_another_edit() {
        let store = Store::memory();
        let settings = Settings {
            auto_learn: false,
            ..Settings::default()
        };
        let a = recorded(&store, "call asha", "Call asha.", "a.wav");
        let b = recorded(&store, "call asha", "Call asha.", "b.wav");
        learn_from_edit(&store, &settings, a, "Call Aasha now.", NOW).unwrap();
        learn_from_edit(&store, &settings, b, "Call Asha later.", NOW).unwrap();
        learn_from_edit(&store, &settings, b, "Call asha.", NOW).unwrap();
        assert_eq!(
            store.examples(Category::Work, 5),
            vec![("call asha".into(), "Call Aasha now.".into())]
        );
        learn_from_edit(&store, &settings, a, "", NOW).unwrap();
        assert!(store.examples(Category::Work, 5).is_empty());
        assert!(store.dictation(a).unwrap().edited.is_none());
    }

    #[test]
    fn overlong_corrections_do_not_create_unacceptable_suggestions() {
        let store = Store::memory();
        let from = ["a".repeat(30), "b".repeat(30), "c".repeat(30)].join(" ");
        let to = ["d".repeat(30), "e".repeat(30), "f".repeat(30)].join(" ");
        for (from, to) in [
            (from.as_str(), "Zephyr"),
            ("Zephyr", to.as_str()),
            (from.as_str(), to.as_str()),
        ] {
            let raw = format!("please ask the team about {from} today");
            let edited = format!("please ask the team about {to} today");
            assert_eq!(corrections(&raw, &edited).len(), 1);
            let id = recorded(&store, &raw, &raw, "a.wav");
            assert!(learn_from_edit(&store, &Settings::default(), id, &edited, NOW)
                .unwrap()
                .is_empty());
            assert!(store.suggestions().unwrap().is_empty());
            assert!(store.dictionary().unwrap().is_empty());
            assert_eq!(store.dictation(id).unwrap().edited.as_deref(), Some(edited.as_str()));
        }
    }
}
