//! `Platform` for a real desktop: the OS adapters behind the core's trait.

use crate::shell;
use speechify_core::app::{Bar, Focus, Platform};
use speechify_core::audio::{self, Chime};
use speechify_core::context::{PasteKeys, Target};
use speechify_core::store::Clock;
use speechify_core::Result;
use speechify_os::capture::Recorder;
use speechify_os::{capture, focus, sound, typing};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

pub struct Desktop {
    pub app: AppHandle,
    pub audio_dir: PathBuf,
    recorder: Option<Recorder>,
    started: Instant,
    muted: bool,
    quitting: Arc<AtomicBool>,
}

/// The wall clock with the local UTC offset.
pub fn clock() -> Clock {
    let now = chrono::Local::now();
    Clock {
        now: now.timestamp(),
        offset_min: now.offset().local_minus_utc() / 60,
    }
}

impl Desktop {
    pub fn new(app: AppHandle, audio_dir: PathBuf, quitting: Arc<AtomicBool>) -> Desktop {
        Desktop {
            app,
            audio_dir,
            recorder: None,
            started: Instant::now(),
            muted: false,
            quitting,
        }
    }
}

impl Drop for Desktop {
    fn drop(&mut self) {
        // Also runs when the worker unwinds: close the microphone and restore
        // output before reporting the failed worker to the UI.
        self.recorder.take();
        if self.muted {
            sound::mute(false);
        }
    }
}

impl Platform for Desktop {
    fn start_capture(&mut self, microphone: &str) -> Result<()> {
        if self.quitting.load(Ordering::SeqCst) {
            return Err("Speechify is quitting.".into());
        }
        let app = self.app.clone();
        let mut last = Instant::now();
        // The bar's waveform: one loudness value about twenty times a second.
        let on_chunk = Box::new(move |chunk: &[f32]| {
            if last.elapsed() >= Duration::from_millis(45) {
                last = Instant::now();
                let _ = app.emit_to("bar", "level", audio::level(chunk));
            }
        });
        self.recorder = Some(Recorder::start(microphone, on_chunk)?);
        Ok(())
    }

    fn stop_capture(&mut self) -> Vec<f32> {
        match self.recorder.take().map(Recorder::stop) {
            Some((samples, rate)) => audio::to_mono_16k(&samples, 1, rate),
            None => vec![],
        }
    }

    fn target(&mut self) -> Target {
        focus::active_window()
            .map(|window| Target {
                app: window.app,
                process: window.process,
                title: window.title,
            })
            .unwrap_or_default()
    }

    fn focus(&mut self) -> Focus {
        match focus::editable() {
            focus::Editable::Yes => Focus::Editable,
            focus::Editable::No => Focus::NotEditable,
            focus::Editable::Unknown => Focus::Unknown,
        }
    }

    fn selection(&mut self) -> String {
        typing::selection()
    }

    fn paste(&mut self, text: &str, keys: PasteKeys, restore_ms: u64) -> Result<()> {
        if self.quitting.load(Ordering::SeqCst) {
            return Err("Speechify is quitting.".into());
        }
        let keys = match keys {
            PasteKeys::Standard => typing::PasteKeys::Standard,
            PasteKeys::WithShift => typing::PasteKeys::WithShift,
            PasteKeys::ShiftInsert => typing::PasteKeys::ShiftInsert,
        };
        typing::paste(text, keys, restore_ms)
    }

    fn copy(&mut self, text: &str) -> Result<()> {
        typing::copy(text)
    }

    fn press_enter(&mut self) {
        if !self.quitting.load(Ordering::SeqCst) {
            typing::press_enter();
        }
    }

    fn chime(&mut self, chime: Chime) {
        capture::play(move |rate| audio::chime(chime, rate));
    }

    fn mute(&mut self, on: bool) {
        sound::mute(on);
        self.muted = on;
    }

    fn show(&mut self, bar: Bar) {
        shell::show_bar(&self.app, &bar);
        // Anything that ends a dictation may have added to the history.
        if matches!(bar, Bar::Done { .. } | Bar::Card { .. }) {
            let _ = self.app.emit("history-changed", ());
        }
    }

    fn save_audio(&mut self, name: &str, samples: &[f32]) -> Result<()> {
        std::fs::create_dir_all(&self.audio_dir).map_err(|e| format!("Could not save the recording: {e}"))?;
        std::fs::write(self.audio_dir.join(name), audio::encode_wav(samples))
            .map_err(|e| format!("Could not save the recording: {e}"))
    }

    fn load_audio(&self, name: &str) -> Result<Vec<f32>> {
        let bytes =
            std::fs::read(self.audio_dir.join(name)).map_err(|e| format!("Could not read the recording: {e}"))?;
        audio::decode_wav(&bytes)
    }

    fn delete_audio(&mut self, name: &str) {
        let _ = std::fs::remove_file(self.audio_dir.join(name));
    }

    fn clock(&self) -> Clock {
        clock()
    }

    fn now_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }
}
