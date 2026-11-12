//! `Engines` for a real desktop: loads and keeps the speech and language models.

use speechify_core::app::Engines;
use speechify_core::asr::{Recognizer, Remote};
use speechify_core::llm::{Chat, Rewriter};
use speechify_core::native::Engine;
use speechify_core::settings::Settings;
use speechify_core::{catalog, Result};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
    everyday: Option<(String, Box<dyn Recognizer>)>,
    chat: Option<(String, Chat)>,
    teacher: Option<Box<dyn Recognizer>>,
    teacher_chat: Option<Chat>,
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

    fn open(&self, model_id: &str, device: &str) -> Result<(Box<dyn Recognizer>, String)> {
        let model = catalog::find(model_id)?;
        if !catalog::installed(&self.models_dir, &model) {
            return Err(format!(
                "{} is not downloaded yet. Open Speechify and download it under Models.",
                model.name
            ));
        }
        let engine = Engine::open(&self.runtime_dir, &catalog::path(&self.models_dir, &model), device)?;
        let device = engine.device().to_string();
        Ok((Box::new(engine), device))
    }

    fn load_everyday(&mut self, settings: &Settings) -> Result<()> {
        let speech = &settings.speech;
        let key = format!(
            "{}|{}|{}|{}|{}",
            speech.model, speech.device, speech.remote_url, speech.remote_model, speech.remote_key
        );
        if self.everyday.as_ref().is_some_and(|(loaded, _)| *loaded == key) {
            return Ok(());
        }
        // Free the old model before loading the new one.
        self.everyday = None;
        let loaded = if speech.remote_url.trim().is_empty() {
            self.open(&speech.model, &speech.device)
        } else {
            Ok((
                Box::new(Remote::new(
                    &speech.remote_url,
                    &speech.remote_key,
                    &speech.remote_model,
                )) as Box<dyn Recognizer>,
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
                self.everyday = Some((key, recognizer));
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
        let _ = self.load_everyday(settings);
    }

    /// Frees the teacher's models once a review is over.
    pub fn release_teacher(&mut self) {
        self.teacher = None;
        self.teacher_chat = None;
    }
}

fn chat(settings: &Settings, model: &str, timeout: Duration) -> Chat {
    Chat::new(&settings.rewrite.url, &settings.rewrite.key, model, timeout)
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
            .map(|(_, recognizer)| recognizer.as_mut())
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
                .models()
                .is_ok_and(|models| models.contains(&settings.teacher.rewrite_model));
        self.teacher_chat = available.then_some(teacher_chat);
        let recognizer = self
            .teacher
            .as_mut()
            .map(|recognizer| recognizer.as_mut())
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
            let _ = warmer.warm();
        });
    }
}
