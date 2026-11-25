//! `Engines` for a real desktop: loads and keeps the speech and language models.

use crate::diagnostics;
use speechify_core::app::Engines;
use speechify_core::asr::{Recognizer, Remote, Shared};
use speechify_core::llm::{Chat, Rewriter};
use speechify_core::native::{Detector, Engine, DETECTOR_FILE};
use speechify_core::settings::Settings;
use speechify_core::{catalog, Result};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Delegates every recognizer operation without logging audio, hints or text.
struct Diagnosed(Box<dyn Recognizer + Send>);

impl Recognizer for Diagnosed {
    fn name(&self) -> &str {
        self.0.name()
    }
    fn transcribe(&mut self, audio: &[f32], hint: &speechify_core::asr::Hint) -> Result<String> {
        diagnostics::attempt("speech.transcribe", || self.0.transcribe(audio, hint))
    }
    fn transcribe_until(
        &mut self,
        audio: &[f32],
        hint: &speechify_core::asr::Hint,
        stop: &std::sync::atomic::AtomicBool,
    ) -> Result<String> {
        diagnostics::attempt("speech.transcribe", || self.0.transcribe_until(audio, hint, stop))
    }
    fn detect(&mut self, audio: &[f32], threshold: f64) -> Option<Result<Vec<std::ops::Range<usize>>>> {
        diagnostics::scope("speech.detect", || {
            let result = self.0.detect(audio, threshold);
            diagnostics::event(
                "speech.detect",
                match &result {
                    Some(Ok(_)) => "ok",
                    Some(Err(_)) => "error",
                    None => "unavailable",
                },
            );
            result
        })
    }
    fn heard(&self) -> Vec<speechify_core::asr::Heard> {
        self.0.heard()
    }
    fn streams(&self) -> bool {
        self.0.streams()
    }
    fn stream_start(&mut self, hint: &speechify_core::asr::Hint, stop: &std::sync::atomic::AtomicBool) -> Result<()> {
        diagnostics::attempt("speech.stream_start", || self.0.stream_start(hint, stop))
    }
    fn stream_feed(
        &mut self,
        audio: &[f32],
        stop: &std::sync::atomic::AtomicBool,
    ) -> Result<speechify_core::asr::Streamed> {
        diagnostics::attempt("speech.stream_feed", || self.0.stream_feed(audio, stop))
    }
    fn stream_end(&mut self, stop: &std::sync::atomic::AtomicBool) -> Result<speechify_core::asr::Streamed> {
        diagnostics::attempt("speech.stream_end", || self.0.stream_end(stop))
    }
}

struct DiagnosedChat(Chat);

impl Rewriter for DiagnosedChat {
    fn rewrite(&self, system: &str, user: &str) -> Result<String> {
        diagnostics::attempt("rewrite", || self.0.rewrite(system, user))
    }
    fn rewrite_timed(&self, system: &str, user: &str) -> Result<(String, speechify_core::trace::ModelTime)> {
        diagnostics::attempt("rewrite", || self.0.rewrite_timed(system, user))
    }
    fn model(&self) -> &str {
        self.0.model()
    }
    fn share(&self) -> Option<Box<dyn Rewriter + Send>> {
        Some(Box::new(DiagnosedChat(self.0.clone())))
    }
}

/// What the UI shows about the speech engine.
#[derive(Debug, Clone, Default)]
pub struct SpeechStatus {
    pub ready: bool,
    pub device: String,
    pub error: String,
}

pub struct Models {
    pub runtime_dir: PathBuf,
    pub models_dir: PathBuf,
    pub status: Arc<Mutex<SpeechStatus>>,
    /// The loaded everyday recogniser and the settings it was loaded for.
    /// Pieces of a recording are recognised with it on threads of their own.
    everyday: Option<(String, Shared<Diagnosed>)>,
    chat: Option<(String, DiagnosedChat)>,
    teacher: Option<Diagnosed>,
    teacher_chat: Option<DiagnosedChat>,
    warmed: Option<Instant>,
}

impl Models {
    pub fn new(runtime_dir: PathBuf, models_dir: PathBuf, status: Arc<Mutex<SpeechStatus>>) -> Models {
        Models {
            runtime_dir,
            models_dir,
            status,
            everyday: None,
            chat: None,
            teacher: None,
            teacher_chat: None,
            warmed: None,
        }
    }

    fn open(&self, model_id: &str, device: &str) -> Result<(Diagnosed, String)> {
        let model = catalog::find(model_id)?;
        if !catalog::installed(&self.models_dir, &model) {
            return Err(format!(
                "{} is not downloaded yet. Open Speechify and download it under Models.",
                model.name
            ));
        }
        let mut engine = diagnostics::attempt("model.open", || {
            Engine::open(&self.runtime_dir, &catalog::path(&self.models_dir, &model), device)
        })?;
        // The voice detector ships with the runtime. Without it, speech is
        // found by loudness, which can miss very quiet speech.
        let detector = self.runtime_dir.join(DETECTOR_FILE);
        if let Ok(found) =
            diagnostics::attempt("detector.open", || Detector::open(&self.runtime_dir, &detector, device))
        {
            engine = engine.with_detector(found);
        }
        let device = engine.device().to_string();
        Ok((Diagnosed(Box::new(engine)), device))
    }

    fn load_everyday(&mut self, settings: &Settings) -> Result<()> {
        let speech = &settings.speech;
        let key = format!(
            "{}|{}|{}|{}|{}",
            speech.active_model(),
            speech.device,
            speech.remote_url,
            speech.remote_model,
            speech.remote_key
        );
        if self.everyday.as_ref().is_some_and(|(loaded, _)| *loaded == key) {
            return Ok(());
        }
        // Free the old model before loading the new one.
        self.everyday = None;
        let loaded = if speech.remote_url.trim().is_empty() {
            self.open(speech.active_model(), &speech.device)
        } else {
            Ok((
                Diagnosed(Box::new(Remote::new(
                    &speech.remote_url,
                    &speech.remote_key,
                    &speech.remote_model,
                ))),
                format!("server {}", speech.remote_url),
            ))
        };
        let mut status = self.status.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        match loaded {
            Ok((recognizer, device)) => {
                *status = SpeechStatus {
                    ready: true,
                    device,
                    error: String::new(),
                };
                self.everyday = Some((key, Shared::new(recognizer)));
                Ok(())
            }
            Err(error) => {
                *status = SpeechStatus {
                    ready: false,
                    device: String::new(),
                    error: error.clone(),
                };
                Err(error)
            }
        }
    }

    /// Loads the everyday model ahead of the first dictation.
    pub fn preload(&mut self, settings: &Settings) {
        let _ = diagnostics::attempt("model.preload", || self.load_everyday(settings));
    }

    /// Frees the teacher's models once a review is over.
    pub fn release_teacher(&mut self) {
        self.teacher = None;
        self.teacher_chat = None;
    }
}

fn chat(settings: &Settings, model: &str, timeout: Duration) -> DiagnosedChat {
    DiagnosedChat(Chat::new(&settings.rewrite.url, &settings.rewrite.key, model, timeout))
}

/// How long dropping the models waits for a piece still being recognised
/// to let go of the model. The exit path waits a little longer than this
/// for the worker, so the process never ends with the model still loaded.
pub const RELEASE_WAIT: Duration = Duration::from_secs(60);

impl Drop for Models {
    /// A piece still being recognised elsewhere holds the model too. The
    /// model must be gone once the worker stops (Metal asserts at exit that
    /// nothing is left on the GPU), so wait for it to let go. The piece was
    /// told to stop, but the runtime cannot stop inside the encoder, so this
    /// can take as long as one piece. The bound only guards against a
    /// runtime that never returns.
    fn drop(&mut self) {
        if let Some((_, recognizer)) = &self.everyday {
            if !recognizer.wait_alone(RELEASE_WAIT) {
                diagnostics::event("model.release", "still in use");
            }
        }
    }
}

impl Engines for Models {
    fn everyday(&mut self, settings: &Settings) -> Result<(&mut dyn Recognizer, Option<&dyn Rewriter>)> {
        self.load_everyday(settings)?;
        let rewrite = &settings.rewrite;
        let key = format!(
            "{}|{}|{}|{}",
            rewrite.url, rewrite.key, rewrite.model, rewrite.timeout_ms
        );
        if !rewrite.enabled {
            self.chat = None;
        } else if self.chat.as_ref().is_none_or(|(built, _)| *built != key) {
            self.chat = Some((
                key,
                chat(
                    settings,
                    &rewrite.model,
                    Duration::from_millis(rewrite.timeout_ms.max(500)),
                ),
            ));
        }
        let recognizer = self
            .everyday
            .as_mut()
            .map(|(_, recognizer)| recognizer as &mut dyn Recognizer)
            .ok_or("The speech model is not loaded")?;
        Ok((recognizer, self.chat.as_ref().map(|(_, chat)| chat as &dyn Rewriter)))
    }

    fn teacher(&mut self, settings: &Settings) -> Result<(&mut dyn Recognizer, Option<&dyn Rewriter>)> {
        if self.teacher.is_none() {
            // The teacher runs on the same device as live dictation.
            self.teacher = Some(self.open(&settings.teacher.speech_model, &settings.speech.device)?.0);
        }
        // The larger language model may take a while per dictation; nobody is waiting.
        let teacher_chat = chat(settings, &settings.teacher.rewrite_model, Duration::from_secs(180));
        let available = settings.rewrite.enabled
            && teacher_chat
                .0
                .models()
                .is_ok_and(|models| models.contains(&settings.teacher.rewrite_model));
        self.teacher_chat = available.then_some(teacher_chat);
        let recognizer = self
            .teacher
            .as_mut()
            .map(|recognizer| recognizer as &mut dyn Recognizer)
            .ok_or("The teacher model is not loaded")?;
        Ok((recognizer, self.teacher_chat.as_ref().map(|chat| chat as &dyn Rewriter)))
    }

    fn warm(&mut self, settings: &Settings) {
        // Ollama unloads idle models; nudge it while the user is still speaking
        // so the cleanup that follows does not wait for a reload.
        let recently = self.warmed.is_some_and(|at| at.elapsed() < Duration::from_secs(240));
        if !settings.rewrite.enabled || recently {
            return;
        }
        self.warmed = Some(Instant::now());
        let warmer = chat(settings, &settings.rewrite.model, Duration::from_secs(60));
        std::thread::spawn(move || {
            let _ = diagnostics::attempt("rewrite.warm", || warmer.0.warm());
        });
    }
}
