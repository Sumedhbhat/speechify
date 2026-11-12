//! The threads behind the window: one polls the keyboard, one runs dictation.

use crate::desktop::Desktop;
use crate::models::{Models, SpeechStatus};
use speechify_core::app::Controller;
use speechify_core::hotkey::{Edge, Key, Tracker};
use speechify_core::session::Event;
use speechify_core::store::Store;
use speechify_os::keys::Keyboard;
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

pub enum Msg {
    Input(Event),
    /// Settings changed: models may need reloading.
    Reload,
    /// Run the nightly review now.
    Teacher,
    /// Explicit cold-start CLI action, applied after model preload.
    StartupToggle,
    Quit,
}

struct Queued {
    message: Msg,
    at: Instant,
}

/// State shared between the worker, the key thread and the UI commands.
pub struct Shared {
    tx: Mutex<Sender<Queued>>,
    /// The UI's own database connection (the worker has another).
    db: Mutex<Store>,
    tracker: Mutex<Option<Tracker>>,
    binding_revision: AtomicUsize,
    pub speech: Arc<Mutex<SpeechStatus>>,
    pub teacher_running: AtomicBool,
    pub quitting: Arc<AtomicBool>,
    /// Number of queued input messages, so a running review yields.
    wanted: AtomicUsize,
    /// Cancel flags for model downloads in progress.
    pub downloads: Mutex<HashMap<String, Arc<AtomicBool>>>,
    pub data_dir: PathBuf,
    pub runtime_dir: PathBuf,
    pub keyboard_available: bool,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Shared {
    pub fn send(&self, msg: Msg) {
        let input = matches!(msg, Msg::Input(_));
        if input {
            // Count queued input rather than resetting a flag at review start:
            // a start already in the channel must also interrupt the review.
            self.wanted.fetch_add(1, Ordering::SeqCst);
        }
        if lock(&self.tx)
            .send(Queued {
                message: msg,
                at: Instant::now(),
            })
            .is_err()
        {
            if input {
                self.wanted.fetch_sub(1, Ordering::SeqCst);
            }
            let mut status = lock(&self.speech);
            status.ready = false;
            status.error = "The dictation worker stopped. Restart Speechify to continue.".into();
        }
    }

    pub fn quit(&self, app: &AppHandle) {
        self.quitting.store(true, Ordering::SeqCst);
        // Let the owning thread close the microphone and restore system sound
        // before exiting. If it has already stopped, its resources are gone.
        if lock(&self.tx)
            .send(Queued {
                message: Msg::Quit,
                at: Instant::now(),
            })
            .is_err()
        {
            app.exit(0);
        }
    }

    pub fn db(&self) -> MutexGuard<'_, Store> {
        lock(&self.db)
    }

    pub fn models_dir(&self) -> PathBuf {
        self.data_dir.join("models")
    }

    pub fn audio_dir(&self) -> PathBuf {
        self.data_dir.join("audio")
    }

    /// Rebuilds the shortcut tracker from settings. Called after a save.
    pub fn rebind(&self) -> speechify_core::Result<()> {
        let tracker = Tracker::new(&self.db().settings().hotkeys)?;
        *lock(&self.tracker) = Some(tracker);
        self.binding_revision.fetch_add(1, Ordering::Release);
        Ok(())
    }
}

/// Where the bundled speech runtime lives. `SPEECHIFY_NATIVE_DIR` overrides
/// it when running from source.
fn runtime_dir(app: &AppHandle) -> PathBuf {
    if let Some(dir) = std::env::var_os("SPEECHIFY_NATIVE_DIR") {
        return PathBuf::from(dir);
    }
    let bundled = app
        .path()
        .resource_dir()
        .map(|dir| dir.join("resources").join("native"))
        .unwrap_or_default();
    if bundled.exists() {
        return bundled;
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources")
        .join("native")
}

pub fn start(app: AppHandle) -> Result<Arc<Shared>, Box<dyn std::error::Error>> {
    let data_dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(&data_dir)?;
    let db_path = data_dir.join("speechify.db");
    let (tx, rx) = channel();
    // The key reader is tied to the thread that creates it, so this one only
    // answers "can we read keys at all?".
    let keyboard_available = Keyboard::new().available();
    let shared = Arc::new(Shared {
        tx: Mutex::new(tx),
        db: Mutex::new(Store::open(&db_path)?),
        tracker: Mutex::new(None),
        binding_revision: AtomicUsize::new(0),
        speech: Arc::new(Mutex::new(SpeechStatus::default())),
        teacher_running: AtomicBool::new(false),
        quitting: Arc::new(AtomicBool::new(false)),
        wanted: AtomicUsize::new(0),
        downloads: Mutex::new(HashMap::new()),
        runtime_dir: runtime_dir(&app),
        data_dir,
        keyboard_available,
    });
    // Unusable shortcuts in a hand-edited database must not stop the app.
    let _ = shared.rebind();

    // Keyboard thread: turns key state into shortcut events.
    let keys = shared.clone();
    std::thread::spawn(move || {
        let keyboard = Keyboard::new();
        if !keyboard.available() {
            return; // No 83 Hz wakeups when global shortcuts cannot work.
        }
        let mut previous = Vec::new();
        let mut revision = 0;
        loop {
            std::thread::sleep(Duration::from_millis(12));
            let names = keyboard.pressed();
            let current_revision = keys.binding_revision.load(Ordering::Acquire);
            if names == previous && current_revision == revision {
                continue;
            }
            revision = current_revision;
            previous.clone_from(&names);
            let pressed: BTreeSet<Key> = names.iter().map(|name| Key::from_name(name)).collect();
            let edges = lock(&keys.tracker)
                .as_mut()
                .map(|tracker| tracker.update(&pressed))
                .unwrap_or_default();
            for edge in edges {
                keys.send(Msg::Input(match edge {
                    Edge::Down(bind) => Event::Down(bind),
                    Edge::Up(bind) => Event::Up(bind),
                    Edge::Released => Event::Released,
                }));
            }
        }
    });

    // Worker thread: owns the microphone, the models and the controller.
    let state = shared.clone();
    std::thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let store = match Store::open(&db_path) {
                Ok(store) => store,
                Err(error) => {
                    let mut status = lock(&state.speech);
                    status.ready = false;
                    status.error = error;
                    return;
                }
            };
            let desktop = Desktop::new(app.clone(), state.audio_dir(), state.quitting.clone());
            let models = Models::new(state.runtime_dir.clone(), state.models_dir(), state.speech.clone());
            let mut controller = Controller::new(desktop, models, store);
            let settings = controller.store.settings();
            controller.engines.preload(&settings);
            let mut checked = Instant::now();
            let mut ticked = Instant::now();
            // Shortcuts received while a blocking operation runs cannot describe
            // the present key state. Never replay them as a new recording.
            let mut accept_after = Instant::now();
            let mut reload = false;
            loop {
                if state.quitting.load(Ordering::SeqCst) {
                    break;
                }
                if reload && controller.is_idle() {
                    controller.engines.preload(&controller.store.settings());
                    reload = false;
                    accept_after = Instant::now();
                }
                let review = match rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(Queued {
                        message: Msg::Input(event),
                        at,
                    }) => {
                        state.wanted.fetch_sub(1, Ordering::SeqCst);
                        if at >= accept_after {
                            let was_idle = controller.is_idle();
                            let began = Instant::now();
                            controller.handle(event);
                            if controller.is_idle() && (!was_idle || began.elapsed() >= Duration::from_millis(100)) {
                                accept_after = Instant::now();
                            }
                        } else if matches!(
                            event,
                            Event::BarClick
                                | Event::Down(
                                    speechify_core::hotkey::Bind::PushToTalk
                                        | speechify_core::hotkey::Bind::HandsFree
                                        | speechify_core::hotkey::Bind::Command
                                )
                        ) {
                            crate::shell::show_bar(
                                &app,
                                &speechify_core::app::Bar::Notice {
                                    message: "Speechify was busy. Try the shortcut again.".into(),
                                    error: false,
                                    undo: false,
                                },
                            );
                        }
                        None
                    }
                    Ok(Queued { message: Msg::Quit, .. }) => break,
                    Ok(Queued {
                        message: Msg::StartupToggle,
                        ..
                    }) => {
                        controller.handle(Event::BarClick);
                        None
                    }
                    Ok(Queued {
                        message: Msg::Reload, ..
                    }) => {
                        reload = true;
                        None
                    }
                    Ok(Queued {
                        message: Msg::Teacher, ..
                    }) => Some(true),
                    Err(RecvTimeoutError::Timeout) => {
                        if !controller.is_idle() {
                            None
                        } else if checked.elapsed() > Duration::from_secs(60) {
                            checked = Instant::now();
                            Some(false)
                        } else {
                            None
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => return,
                };
                if ticked.elapsed() >= Duration::from_millis(100) && !controller.is_idle() {
                    controller.handle(Event::Tick);
                    ticked = Instant::now();
                    if controller.is_idle() {
                        accept_after = Instant::now();
                    }
                }
                if let Some(forced) = review {
                    state.teacher_running.store(true, Ordering::Relaxed);
                    let result = if state.wanted.load(Ordering::SeqCst) == 0 {
                        controller.review_if_due(forced, &|| {
                            state.wanted.load(Ordering::SeqCst) == 0 && !state.quitting.load(Ordering::SeqCst)
                        })
                    } else {
                        None
                    };
                    controller.engines.release_teacher();
                    if result.is_some() {
                        accept_after = Instant::now();
                    }
                    state.teacher_running.store(false, Ordering::Relaxed);
                    if let Some(result) = result {
                        let payload = match result {
                            Ok(report) => serde_json::json!({ "report": report, "error": "" }),
                            Err(error) => serde_json::json!({ "report": null, "error": error }),
                        };
                        let _ = app.emit("teacher-finished", payload);
                        let _ = app.emit("history-changed", ());
                    } else if forced {
                        let _ = app.emit(
                            "teacher-finished",
                            serde_json::json!({ "report": null, "error": "Finish dictating first, then try again." }),
                        );
                    }
                }
            }
        }));
        state.teacher_running.store(false, Ordering::Relaxed);
        if state.quitting.load(Ordering::SeqCst) {
            app.exit(0);
        } else if result.is_err() {
            let message = "The dictation worker stopped. Restart Speechify to continue.";
            {
                let mut status = lock(&state.speech);
                status.ready = false;
                status.error = message.into();
            }
            crate::shell::show_bar(
                &app,
                &speechify_core::app::Bar::Notice {
                    message: message.into(),
                    error: true,
                    undo: false,
                },
            );
        }
    });
    Ok(shared)
}
