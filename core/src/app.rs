//! The dictation controller: what happens when a shortcut is pressed.
//!
//! It owns the session state machine and drives the pipeline, and reaches
//! the outside world only through `Platform` and `Engines`, so the whole
//! behaviour of the app is tested here without a microphone or a screen.

use crate::asr::Recognizer;
use crate::audio::Chime;
use crate::context::{categorize, restore_delay_ms, Os, PasteKeys, Target};
use crate::llm::Rewriter;
use crate::pipeline::{self, Outcome, Request, RewriteCircuit};
use crate::session::{Effect, Event, Notice, Session};
use crate::settings::{Retention, Settings};
use crate::store::{self, Clock, Store};
use crate::{teacher, Result};
use serde::Serialize;

/// Whether the focused control accepts typed text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Editable,
    /// Something is focused, but it is not a text field.
    NotEditable,
    /// The platform cannot tell; pasting is attempted.
    Unknown,
}

/// What the floating bar shows.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Bar {
    Idle,
    Listening {
        command: bool,
        locked: bool,
    },
    Processing,
    /// Text was pasted.
    Done {
        words: u32,
    },
    /// Text could not be pasted; offer it for copying.
    Card {
        text: String,
        message: String,
    },
    Notice {
        message: String,
        error: bool,
        undo: bool,
    },
}

/// Everything the controller needs from the operating system.
pub trait Platform {
    fn start_capture(&mut self, microphone: &str) -> Result<()>;
    /// Stops recording and returns 16 kHz mono samples.
    fn stop_capture(&mut self) -> Vec<f32>;
    fn target(&mut self) -> Target;
    fn focus(&mut self) -> Focus;
    /// The selected text in the focused app, for Command Mode.
    fn selection(&mut self) -> String;
    /// Types `text` into the focused app via the clipboard, restoring the
    /// previous clipboard after `restore_ms`.
    fn paste(&mut self, text: &str, keys: PasteKeys, restore_ms: u64) -> Result<()>;
    fn copy(&mut self, text: &str) -> Result<()>;
    fn press_enter(&mut self);
    fn chime(&mut self, chime: Chime);
    /// Mutes or restores system audio output.
    fn mute(&mut self, on: bool);
    fn show(&mut self, bar: Bar);
    fn save_audio(&mut self, name: &str, samples: &[f32]) -> Result<()>;
    fn load_audio(&self, name: &str) -> Result<Vec<f32>>;
    fn delete_audio(&mut self, name: &str);
    fn clock(&self) -> Clock;
    /// A monotonic clock in milliseconds.
    fn now_ms(&self) -> u64;
}

/// The loaded models. Loading is the shell's business; it may be slow.
pub trait Engines {
    /// The everyday recogniser and, if enabled and reachable, the rewriter.
    fn everyday(&mut self, settings: &Settings) -> Result<(&mut dyn Recognizer, Option<&dyn Rewriter>)>;
    /// The larger models for the nightly review.
    fn teacher(&mut self, settings: &Settings) -> Result<(&mut dyn Recognizer, Option<&dyn Rewriter>)>;
    /// Called when recording starts, to get models into memory early.
    fn warm(&mut self, settings: &Settings);
}

pub struct Controller<P: Platform, E: Engines> {
    pub platform: P,
    pub engines: E,
    pub store: Store,
    session: Session,
    target: Target,
    command: bool,
    /// The last cancelled or failed recording, for Undo and retry.
    kept: Option<(Vec<f32>, Target, bool)>,
    muted: bool,
    rewrite_circuit: RewriteCircuit,
    rewrite_settings: crate::settings::Rewrite,
}

impl<P: Platform, E: Engines> Controller<P, E> {
    pub fn new(platform: P, engines: E, store: Store) -> Self {
        let rewrite_settings = store.settings().rewrite;
        Self {
            platform,
            engines,
            store,
            session: Session::new(),
            target: Target::default(),
            command: false,
            kept: None,
            muted: false,
            rewrite_circuit: RewriteCircuit::default(),
            rewrite_settings,
        }
    }

    pub fn is_idle(&self) -> bool {
        !self.session.is_recording() && !self.session.is_busy()
    }

    /// Status consumers can explain why cleanup currently uses only rules.
    pub fn rewrite_retry_at_ms(&self) -> Option<u64> {
        self.rewrite_circuit
            .cooling(self.platform.now_ms())
            .then_some(self.rewrite_circuit.retry_at_ms)
    }

    fn sync_rewrite_settings(&mut self, settings: &Settings) {
        if self.rewrite_settings != settings.rewrite {
            self.rewrite_circuit = RewriteCircuit::default();
            self.rewrite_settings = settings.rewrite.clone();
        }
    }

    fn notice(&mut self, message: &str, error: bool, undo: bool) {
        self.platform.show(Bar::Notice {
            message: message.into(),
            error,
            undo,
        });
    }

    fn unmute(&mut self) {
        if self.muted {
            self.platform.mute(false);
            self.muted = false;
        }
    }

    /// Feeds one input event through the state machine and carries out
    /// whatever it decides.
    pub fn handle(&mut self, event: Event) {
        let settings = self.store.settings();
        for effect in self.session.handle(event, self.platform.now_ms()) {
            match effect {
                Effect::Start => {
                    if !self.start(&settings) {
                        // The microphone did not open: skip the "listening" display.
                        break;
                    }
                }
                Effect::Mode { command, locked } => {
                    self.command = command;
                    self.platform.show(Bar::Listening { command, locked });
                }
                Effect::Stop { command } => {
                    let audio = self.platform.stop_capture();
                    self.unmute();
                    if settings.sounds {
                        self.platform.chime(Chime::Stop);
                    }
                    let target = self.target.clone();
                    self.process(&settings, audio, target, command);
                    self.session.handle(Event::Finished, self.platform.now_ms());
                }
                Effect::Discard { keep } => {
                    let audio = self.platform.stop_capture();
                    self.unmute();
                    self.kept = keep.then(|| (audio, self.target.clone(), self.command));
                    self.platform.show(Bar::Idle);
                }
                Effect::Retry => {
                    if let Some((audio, target, command)) = self.kept.take() {
                        self.process(&settings, audio, target, command);
                    }
                    self.session.handle(Event::Finished, self.platform.now_ms());
                }
                Effect::PasteLast => self.paste_last(),
                Effect::CopyLast => self.copy_last(),
                Effect::Notice(Notice::StillProcessing) => {
                    self.notice("Still working on your last dictation", false, false)
                }
                Effect::Notice(Notice::LimitWarning) => {
                    self.notice("One minute left: dictations can be up to 20 minutes", false, false)
                }
                Effect::Notice(Notice::LimitReached) => {
                    self.notice("Reached the 20-minute limit; transcribing what you said", false, false)
                }
                Effect::Notice(Notice::Cancelled) => self.notice("Dictation cancelled", false, true),
            }
        }
    }

    fn start(&mut self, settings: &Settings) -> bool {
        // Remember where the text is headed before the bar or anything else can take focus.
        self.target = self.platform.target();
        if let Err(error) = self.platform.start_capture(&settings.microphone) {
            self.session = Session::new();
            self.platform.chime(Chime::Error);
            self.notice(&error, true, false);
            return false;
        }
        if settings.mute_while_dictating {
            self.platform.mute(true);
            self.muted = true;
        }
        if settings.sounds {
            self.platform.chime(Chime::Start);
        }
        self.sync_rewrite_settings(settings);
        let mut warm = settings.clone();
        warm.rewrite.enabled &= !self.rewrite_circuit.cooling(self.platform.now_ms()) || self.command;
        self.engines.warm(&warm);
        true
    }

    fn process(&mut self, settings: &Settings, audio: Vec<f32>, target: Target, command: bool) {
        self.platform.show(Bar::Processing);
        let selection = if command { self.platform.selection() } else { String::new() };
        let clock = self.platform.clock();
        let request = Request {
            audio: &audio,
            target: target.clone(),
            command,
            selection: &selection,
            clock,
        };
        self.sync_rewrite_settings(settings);
        let now_ms = self.platform.now_ms();
        let mut active = settings.clone();
        active.rewrite.enabled &= !self.rewrite_circuit.cooling(now_ms) || command;
        let outcome = self.engines.everyday(&active).and_then(|(recognizer, rewriter)| {
            pipeline::run_guarded(
                &request,
                &active,
                &self.store,
                recognizer,
                rewriter,
                (&mut self.rewrite_circuit, now_ms),
            )
        });
        match outcome {
            Ok(Outcome::Silent) => self.notice("No speech heard. Is your microphone muted?", true, false),
            Ok(Outcome::Nothing) => self.notice("Didn't catch that", false, false),
            Ok(Outcome::Text {
                paste,
                mut record,
                press_enter,
            }) => {
                let pasted = self.deliver(&paste, &target, press_enter);
                record.outcome = if pasted { store::Outcome::Pasted } else { store::Outcome::Copied };
                // A bare "press enter" typed nothing, so there is nothing to keep.
                let typed = !record.text.is_empty();
                let _ = typed && self.remember(settings, &mut record, &audio, clock);
                if pasted {
                    self.platform.show(Bar::Done { words: record.words });
                }
            }
            Err(error) => {
                if settings.sounds {
                    self.platform.chime(Chime::Error);
                }
                // Keep the recording so nothing said is lost to a failure.
                self.kept = Some((audio, target, command));
                self.session.offer_undo();
                self.notice(&error, true, true);
            }
        }
    }

    /// Pastes into the focused field, or shows the copy card when there is
    /// nowhere to paste. Returns whether the text was pasted.
    fn deliver(&mut self, text: &str, target: &Target, press_enter: bool) -> bool {
        let category = categorize(target, &self.store.rules());
        let os = Os::current();
        let shown = text.trim_end().to_string();
        if !text.is_empty() {
            if self.platform.focus() == Focus::NotEditable {
                self.platform.show(Bar::Card {
                    text: shown,
                    message: "Select a text box first, then dictate".into(),
                });
                return false;
            }
            if let Err(error) = self
                .platform
                .paste(text, category.paste_keys(os), restore_delay_ms(target, os))
            {
                self.platform.show(Bar::Card {
                    text: shown,
                    message: error,
                });
                return false;
            }
        }
        if press_enter {
            self.platform.press_enter();
        }
        true
    }

    /// Saves the dictation and its recording. Returns whether it was stored.
    fn remember(&mut self, settings: &Settings, record: &mut store::Dictation, audio: &[f32], clock: Clock) -> bool {
        if settings.retention == Retention::Never {
            return false;
        }
        if settings.keep_audio {
            let name = format!("{}-{}.wav", clock.now, self.platform.now_ms() % 100_000);
            if self
                .platform
                .save_audio(&name, &crate::audio::trim(audio).samples)
                .is_ok()
            {
                record.audio = name;
            }
        }
        let _ = self.store.add_dictation(record);
        for stale in self.store.prune(settings.retention, clock).unwrap_or_default() {
            self.platform.delete_audio(&stale);
        }
        true
    }

    fn paste_last(&mut self) {
        let Some(last) = self.store.last() else {
            return self.notice("Nothing to paste yet", false, false);
        };
        let target = self.platform.target();
        if self.deliver(&last.text, &target, false) {
            let _ = self.store.set_outcome(last.id, store::Outcome::Pasted);
        }
    }

    fn copy_last(&mut self) {
        let copied = self.store.last().map(|last| self.platform.copy(&last.text));
        match copied {
            Some(Ok(())) => self.notice("Copied your last dictation", false, false),
            Some(Err(error)) => self.notice(&error, true, false),
            None => self.notice("Nothing to copy yet", false, false),
        }
    }

    /// Runs the nightly review if it is due and the app is idle. Returns the
    /// report when a review ran. `keep_going` lets the shell stop it the
    /// moment the user wants to dictate.
    pub fn review_if_due(&mut self, force: bool, keep_going: &dyn Fn() -> bool) -> Option<Result<teacher::Report>> {
        let settings = self.store.settings();
        let clock = self.platform.clock();
        let last_run = self.store.get("teacher_last_run").and_then(|value| value.parse().ok());
        if !self.is_idle()
            || settings.retention == Retention::Never
            || !(force || teacher::due(&settings, last_run, clock))
        {
            return None;
        }
        // A day with nothing to review should not load the large models.
        if self.store.pending_review(1).unwrap_or_default().is_empty() {
            let _ = self.store.set("teacher_last_run", &clock.now.to_string());
            return Some(Ok(teacher::Report::default()));
        }
        let platform = &self.platform;
        let result = self.engines.teacher(&settings).and_then(|(recognizer, rewriter)| {
            teacher::review(
                &self.store,
                &settings,
                recognizer,
                rewriter,
                &|name| platform.load_audio(name),
                clock,
                keep_going,
            )
        });
        if let Ok(report) = &result {
            let _ = self.store.set("teacher_last_run", &clock.now.to_string());
            let _ = self.store.set(
                "teacher_last_report",
                &serde_json::to_string(report).unwrap_or_default(),
            );
        }
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asr::Fixed;
    use crate::audio::tone;
    use crate::hotkey::Bind;
    use crate::llm::Scripted;

    #[derive(Default)]
    struct Fake {
        log: Vec<String>,
        bars: Vec<Bar>,
        audio: Vec<f32>,
        focus: Option<Focus>,
        now: u64,
        mic_error: Option<String>,
        paste_error: Option<String>,
        copy_error: Option<String>,
        saved: Vec<String>,
        save_fails: bool,
    }

    impl Platform for Fake {
        fn start_capture(&mut self, microphone: &str) -> Result<()> {
            self.log.push(format!("start:{microphone}"));
            self.mic_error.clone().map_or(Ok(()), Err)
        }
        fn stop_capture(&mut self) -> Vec<f32> {
            self.log.push("stop".into());
            self.audio.clone()
        }
        fn target(&mut self) -> Target {
            Target {
                app: "Slack".into(),
                process: "slack.exe".into(),
                title: "general".into(),
            }
        }
        fn focus(&mut self) -> Focus {
            self.focus.unwrap_or(Focus::Editable)
        }
        fn selection(&mut self) -> String {
            "selected words".into()
        }
        fn paste(&mut self, text: &str, keys: PasteKeys, restore_ms: u64) -> Result<()> {
            // The clipboard-restore delay differs by operating system; the tests
            // only care that the controller asked for the right one.
            let usual = restore_ms == restore_delay_ms(&self.target(), Os::current());
            let delay = ["unusual delay", "usual delay"][usize::from(usual)];
            self.log.push(format!("paste:{text}|{keys:?}|{delay}"));
            self.paste_error.clone().map_or(Ok(()), Err)
        }
        fn copy(&mut self, text: &str) -> Result<()> {
            self.log.push(format!("copy:{text}"));
            self.copy_error.clone().map_or(Ok(()), Err)
        }
        fn press_enter(&mut self) {
            self.log.push("enter".into());
        }
        fn chime(&mut self, chime: Chime) {
            self.log.push(format!("chime:{chime:?}"));
        }
        fn mute(&mut self, on: bool) {
            self.log.push(format!("mute:{on}"));
        }
        fn show(&mut self, bar: Bar) {
            self.bars.push(bar);
        }
        fn save_audio(&mut self, name: &str, samples: &[f32]) -> Result<()> {
            if self.save_fails {
                return Err("disk full".into());
            }
            self.saved.push(format!("{name}:{}", samples.len()));
            Ok(())
        }
        fn load_audio(&self, name: &str) -> Result<Vec<f32>> {
            Ok(tone(1000 + name.len(), 0.3))
        }
        fn delete_audio(&mut self, name: &str) {
            self.log.push(format!("delete:{name}"));
        }
        fn clock(&self) -> Clock {
            Clock {
                now: 1_800_000_000 + self.now as i64 / 1000,
                offset_min: 330,
            }
        }
        fn now_ms(&self) -> u64 {
            self.now
        }
    }

    struct Models {
        asr: Fixed,
        llm: Option<Scripted>,
        teacher_asr: Fixed,
        broken: Option<String>,
        warmed: u32,
    }

    impl Engines for Models {
        fn everyday(&mut self, _: &Settings) -> Result<(&mut dyn Recognizer, Option<&dyn Rewriter>)> {
            if let Some(error) = &self.broken {
                return Err(error.clone());
            }
            Ok((&mut self.asr, self.llm.as_ref().map(|llm| llm as &dyn Rewriter)))
        }
        fn teacher(&mut self, _: &Settings) -> Result<(&mut dyn Recognizer, Option<&dyn Rewriter>)> {
            if let Some(error) = &self.broken {
                return Err(error.clone());
            }
            Ok((&mut self.teacher_asr, None))
        }
        fn warm(&mut self, _: &Settings) {
            self.warmed += 1;
        }
    }

    fn app(heard: &[&str]) -> Controller<Fake, Models> {
        let store = Store::memory();
        let mut settings = Settings::default();
        settings.rewrite.enabled = false;
        store.save_settings(&settings).unwrap();
        let platform = Fake {
            audio: tone(2000, 0.3),
            ..Fake::default()
        };
        Controller::new(
            platform,
            Models {
                asr: Fixed::new(heard),
                llm: None,
                teacher_asr: Fixed::new(&[]),
                broken: None,
                warmed: 0,
            },
            store,
        )
    }

    fn dictate(app: &mut Controller<Fake, Models>) {
        app.handle(Event::Down(Bind::PushToTalk));
        app.platform.now += 2000;
        app.handle(Event::Up(Bind::PushToTalk));
    }

    fn set(app: &mut Controller<Fake, Models>, change: impl Fn(&mut Settings)) {
        let mut settings = app.store.settings();
        change(&mut settings);
        app.store.save_settings(&settings).unwrap();
    }

    #[test]
    fn hold_speak_release_pastes_and_records() {
        let mut app = app(&["hello team the build is green"]);
        dictate(&mut app);
        assert_eq!(
            app.platform.log,
            vec![
                "start:",
                "chime:Start",
                "stop",
                "chime:Stop",
                "paste:Hello team the build is green. |Standard|usual delay"
            ]
        );
        assert_eq!(
            app.platform.bars,
            vec![
                Bar::Listening {
                    command: false,
                    locked: false
                },
                Bar::Processing,
                Bar::Done { words: 6 }
            ]
        );
        assert_eq!(app.engines.warmed, 1);
        assert!(app.is_idle());
        let saved = app.store.last().unwrap();
        assert_eq!(
            (saved.app.as_str(), saved.outcome, saved.words),
            ("Slack", store::Outcome::Pasted, 6)
        );
        assert!(saved.audio.ends_with(".wav"));
        assert_eq!(app.platform.saved.len(), 1);
    }

    #[test]
    fn with_no_text_field_the_bar_offers_the_text_to_copy() {
        let mut app = app(&["note to self buy milk", "second try works"]);
        app.platform.focus = Some(Focus::NotEditable);
        dictate(&mut app);
        assert_eq!(
            app.platform.bars.last().unwrap(),
            &Bar::Card {
                text: "Note to self buy milk.".into(),
                message: "Select a text box first, then dictate".into()
            }
        );
        assert!(!app.platform.log.iter().any(|entry| entry.starts_with("paste")));
        assert_eq!(app.store.last().unwrap().outcome, store::Outcome::Copied);

        // Later, with a field focused, "paste last" delivers it.
        app.platform.focus = Some(Focus::Unknown);
        app.handle(Event::Down(Bind::PasteLast));
        app.handle(Event::Released);
        assert_eq!(
            app.platform.log.last().unwrap(),
            "paste:Note to self buy milk.|Standard|usual delay"
        );
        assert_eq!(app.store.last().unwrap().outcome, store::Outcome::Pasted);

        // A paste that fails also falls back to the card.
        app.platform.paste_error = Some("The clipboard is busy".into());
        dictate(&mut app);
        assert_eq!(
            app.platform.bars.last().unwrap(),
            &Bar::Card {
                text: "Second try works.".into(),
                message: "The clipboard is busy".into()
            }
        );
    }

    #[test]
    fn copy_and_paste_last_with_and_without_history() {
        let mut app = app(&["first words spoken here"]);
        app.handle(Event::Down(Bind::CopyLast));
        app.handle(Event::Down(Bind::PasteLast));
        app.handle(Event::Released);
        let messages: Vec<String> = app.platform.bars.iter().map(|bar| format!("{bar:?}")).collect();
        assert!(messages[0].contains("Nothing to copy yet") && messages[1].contains("Nothing to paste yet"));
        dictate(&mut app);
        app.handle(Event::Down(Bind::CopyLast));
        assert_eq!(app.platform.log.last().unwrap(), "copy:First words spoken here.");
        assert!(format!("{:?}", app.platform.bars.last().unwrap()).contains("Copied your last dictation"));
        app.platform.copy_error = Some("The clipboard is busy".into());
        app.handle(Event::Down(Bind::CopyLast));
        assert_eq!(
            app.platform.bars.last().unwrap(),
            &Bar::Notice {
                message: "The clipboard is busy".into(),
                error: true,
                undo: false
            }
        );
    }

    #[test]
    fn cancel_then_undo_transcribes_the_kept_recording() {
        let mut app = app(&["this was nearly lost"]);
        app.handle(Event::Down(Bind::PushToTalk));
        app.platform.now += 900;
        app.handle(Event::Down(Bind::Cancel));
        assert_eq!(
            app.platform.bars[1..],
            [
                Bar::Idle,
                Bar::Notice {
                    message: "Dictation cancelled".into(),
                    error: false,
                    undo: true
                }
            ]
        );
        assert!(app.store.last().is_none());
        app.handle(Event::Undo);
        assert!(app
            .platform
            .log
            .last()
            .unwrap()
            .starts_with("paste:This was nearly lost."));
        assert!(app.is_idle());
        // A stray tap keeps nothing, so Undo afterwards does nothing.
        app.handle(Event::Down(Bind::PushToTalk));
        app.platform.now += 100;
        app.handle(Event::Up(Bind::PushToTalk));
        app.platform.now += 600;
        app.handle(Event::Tick);
        let before = app.platform.log.len();
        app.handle(Event::Undo);
        assert_eq!(app.platform.log.len(), before);
    }

    #[test]
    fn silence_noise_and_failures_are_explained() {
        let mut app = app(&["[BLANK_AUDIO]"]);
        app.platform.audio = vec![0.0; 32_000];
        dictate(&mut app);
        assert_eq!(
            app.platform.bars.last().unwrap(),
            &Bar::Notice {
                message: "No speech heard. Is your microphone muted?".into(),
                error: true,
                undo: false
            }
        );
        app.platform.audio = tone(2000, 0.3);
        dictate(&mut app);
        assert_eq!(
            app.platform.bars.last().unwrap(),
            &Bar::Notice {
                message: "Didn't catch that".into(),
                error: false,
                undo: false
            }
        );

        // The speech engine failing keeps the audio for a retry.
        app.engines.broken = Some("The speech model is still downloading".into());
        dictate(&mut app);
        assert_eq!(
            app.platform.bars.last().unwrap(),
            &Bar::Notice {
                message: "The speech model is still downloading".into(),
                error: true,
                undo: true
            }
        );
        assert_eq!(app.platform.log.last().unwrap(), "chime:Error");
        app.engines.broken = None;
        app.engines.asr = Fixed::new(&["now it works fine"]);
        app.handle(Event::Undo);
        assert!(app.platform.log.last().unwrap().starts_with("paste:Now it works fine."));

        // A microphone that will not open resets the session.
        app.platform.mic_error = Some("No microphone found".into());
        app.handle(Event::Down(Bind::PushToTalk));
        assert_eq!(
            app.platform.bars.last().unwrap(),
            &Bar::Notice {
                message: "No microphone found".into(),
                error: true,
                undo: false
            }
        );
        assert!(app.is_idle());
    }

    #[test]
    fn settings_shape_the_session() {
        let mut app = app(&["search for flights press enter", "quiet one"]);
        set(&mut app, |s| {
            s.sounds = false;
            s.mute_while_dictating = true;
            s.press_enter = true;
            s.microphone = "USB Mic".into();
            s.keep_audio = false;
        });
        dictate(&mut app);
        assert_eq!(
            app.platform.log,
            vec![
                "start:USB Mic",
                "mute:true",
                "stop",
                "mute:false",
                "paste:Search for flights. |Standard|usual delay",
                "enter"
            ]
        );
        assert_eq!(app.store.last().unwrap().audio, "");

        // "Press enter" on its own presses Enter and pastes nothing.
        app.engines.asr = Fixed::new(&["press enter", "quiet one"]);
        dictate(&mut app);
        assert_eq!(app.platform.log.last().unwrap(), "enter");
        assert!(!app.platform.log[6..].iter().any(|entry| entry.starts_with("paste")));

        // "Never store" keeps no history at all.
        set(&mut app, |s| s.retention = Retention::Never);
        dictate(&mut app);
        assert_eq!(app.store.all().unwrap().len(), 1);
    }

    #[test]
    fn retention_removes_old_recordings_and_survives_a_full_disk() {
        let mut app = app(&["fresh words today", "more words"]);
        let old = store::Dictation {
            audio: "old.wav".into(),
            created_at: 1_700_000_000,
            text: "old".into(),
            ..store::Dictation::default()
        };
        app.store.add_dictation(&old).unwrap();
        set(&mut app, |s| s.retention = Retention::Day);
        dictate(&mut app);
        assert!(app.platform.log.contains(&"delete:old.wav".to_string()));
        app.platform.save_fails = true;
        dictate(&mut app);
        assert_eq!(app.store.last().unwrap().audio, "");
    }

    #[test]
    fn hands_free_notices_and_limits() {
        let mut app = app(&["long talk"]);
        app.handle(Event::BarClick);
        assert_eq!(
            app.platform.bars.last().unwrap(),
            &Bar::Listening {
                command: false,
                locked: true
            }
        );
        app.platform.now += crate::session::WARN_MS;
        app.handle(Event::Tick);
        assert!(format!("{:?}", app.platform.bars.last().unwrap()).contains("One minute left"));
        app.platform.now += 60_000;
        app.handle(Event::Tick);
        assert!(app
            .platform
            .bars
            .iter()
            .any(|bar| format!("{bar:?}").contains("20-minute limit")));
        assert!(app.is_idle());
    }

    #[test]
    fn command_mode_sends_the_selection_to_the_model() {
        let mut app = app(&["make it formal"]);
        app.engines.llm = Some(Scripted::new(vec![Ok("Selected words, formally.".into())]));
        app.handle(Event::Down(Bind::Command));
        assert_eq!(
            app.platform.bars.last().unwrap(),
            &Bar::Listening {
                command: true,
                locked: false
            }
        );
        app.platform.now += 1500;
        app.handle(Event::Up(Bind::Command));
        assert_eq!(
            app.platform.log.last().unwrap(),
            "paste:Selected words, formally.|Standard|usual delay"
        );
        assert!(app.engines.llm.as_ref().unwrap().seen.borrow()[0]
            .1
            .contains("selected words"));
        assert!(app.store.last().unwrap().command);
    }

    #[test]
    fn controller_skips_a_failing_rewriter_and_resets_on_settings_changes() {
        let raw = "please send the report to the team tomorrow";
        let mut app = app(&[raw, raw, raw, raw, raw, raw, raw]);
        set(&mut app, |s| s.rewrite.enabled = true);
        app.engines.llm = Some(Scripted::new(vec![
            Err("timeout".into()),
            Err("timeout".into()),
            Ok("Please send the report to the team tomorrow.".into()),
            Err("timeout".into()),
            Err("timeout".into()),
            Ok("Please send the report to the team tomorrow.".into()),
        ]));
        dictate(&mut app);
        assert_eq!(app.rewrite_retry_at_ms(), None);
        dictate(&mut app);
        let retry = app.rewrite_retry_at_ms().unwrap();
        dictate(&mut app);
        assert_eq!(app.engines.llm.as_ref().unwrap().seen.borrow().len(), 2);
        assert!(!app.store.last().unwrap().rewritten);
        app.platform.now = retry;
        assert_eq!(app.rewrite_retry_at_ms(), None);
        dictate(&mut app);
        assert!(app.store.last().unwrap().rewritten);
        dictate(&mut app);
        dictate(&mut app);
        assert!(app.rewrite_retry_at_ms().is_some());
        set(&mut app, |s| s.rewrite.model = "replacement model".into());
        dictate(&mut app);
        assert!(app.store.last().unwrap().rewritten);
        assert_eq!(app.rewrite_retry_at_ms(), None);
    }

    #[test]
    fn busy_notice_reaches_the_bar() {
        let mut app = app(&[]);
        app.session.handle(Event::Down(Bind::HandsFree), 0);
        app.session.handle(Event::BarClick, 5000);
        app.handle(Event::Down(Bind::PushToTalk));
        assert_eq!(
            app.platform.bars,
            vec![Bar::Notice {
                message: "Still working on your last dictation".into(),
                error: false,
                undo: false
            }]
        );
        assert!(!app.is_idle());
        assert!(app.review_if_due(true, &|| true).is_none());
    }

    #[test]
    fn the_nightly_review_runs_when_due() {
        let mut app = app(&["ask sumit about the launch plan", "tell sumit it shipped on time"]);
        dictate(&mut app);
        dictate(&mut app);
        // 1.8e9 s is 03:50 in India: past the 2 am default.
        app.engines.teacher_asr = Fixed::new(&["Ask Sumedh about the launch plan.", "Tell Sumedh it shipped on time."]);
        let report = app.review_if_due(false, &|| true).unwrap().unwrap();
        assert_eq!(report.reviewed, 2);
        assert_eq!(report.learned.len(), 1);
        assert_eq!(app.store.dictionary().unwrap()[0].term, "Sumedh");
        assert!(app.store.get("teacher_last_report").unwrap().contains("\"reviewed\":2"));
        // Not due again today, unless forced; then there is nothing left to review.
        assert!(app.review_if_due(false, &|| true).is_none());
        assert_eq!(
            app.review_if_due(true, &|| true).unwrap().unwrap(),
            teacher::Report::default()
        );

        app.engines.asr = Fixed::new(&["another one for tonight"]);
        dictate(&mut app);
        app.engines.broken = Some("The teacher model is not downloaded".into());
        assert_eq!(
            app.review_if_due(true, &|| true).unwrap().unwrap_err(),
            "The teacher model is not downloaded"
        );
        set(&mut app, |s| s.retention = Retention::Never);
        assert!(app.review_if_due(true, &|| true).is_none());
    }
}
