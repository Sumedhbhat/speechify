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
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
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

/// The longest Settings may hold shortcuts off while recording one.
const PAUSE_LIMIT: Duration = Duration::from_secs(30);

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
    /// Settings is recording a new shortcut: the keys are for it, not for
    /// dictation. Expires on its own so a closed page cannot leave every
    /// shortcut switched off.
    shortcuts_paused_until: Mutex<Option<Instant>>,
    pub speech: Arc<Mutex<SpeechStatus>>,
    pub teacher_running: AtomicBool,
    pub quitting: Arc<AtomicBool>,
    /// The process is already exiting, so the worker must not ask it to.
    exiting: AtomicBool,
    /// Set once the worker has stopped and dropped the speech models.
    stopped: (Mutex<bool>, Condvar),
    /// Number of queued input messages, so a running review yields.
    wanted: AtomicUsize,
    /// Cancel flags for model downloads in progress.
    pub downloads: Mutex<HashMap<String, Arc<AtomicBool>>>,
    pub data_dir: PathBuf,
    pub runtime_dir: PathBuf,
    pub keyboard_available: AtomicBool,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Shared {
    pub fn send(&self, msg: Msg) {
        let input = matches!(msg, Msg::Input(_));
        if let Msg::Input(event) = &msg {
            if !matches!(event, Event::Tick) {
                // Events contain binding names, never the raw keys being typed.
                crate::diagnostics::event("input.queued", &format!("{event:?}"));
            }
        }
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
        crate::diagnostics::event("app.quit", "worker_requested");
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

    /// Stops the worker and waits until it has dropped the speech models.
    ///
    /// ggml's Metal backend asserts at process exit that no GPU buffers are
    /// left (`GGML_ASSERT([rsets->data count] == 0)` in a static destructor).
    /// A model still loaded then aborts the process, and macOS reports the
    /// quit as a crash. Quits that skip `quit` (Dock › Quit, logging out, an
    /// installer's or script's quit Apple event, an update's restart) all
    /// reach `RunEvent::Exit` first, which calls this.
    pub fn release_for_exit(&self, wait: Duration) {
        self.exiting.store(true, Ordering::SeqCst);
        self.quitting.store(true, Ordering::SeqCst);
        let _ = lock(&self.tx).send(Queued {
            message: Msg::Quit,
            at: Instant::now(),
        });
        let (stopped, signal) = &self.stopped;
        let (stopped, _) = signal
            .wait_timeout_while(lock(stopped), wait, |stopped| !*stopped)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        crate::diagnostics::event("app.release_models", if *stopped { "released" } else { "timed out" });
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

    /// Stops reading shortcuts while Settings records a new one, for up to
    /// `PAUSE_LIMIT`.
    pub fn pause_shortcuts(&self, paused: bool) {
        *lock(&self.shortcuts_paused_until) = paused.then(|| Instant::now() + PAUSE_LIMIT);
    }

    fn shortcuts_paused(&self) -> bool {
        lock(&self.shortcuts_paused_until).is_some_and(|until| Instant::now() < until)
    }

    /// Rebuilds the shortcut tracker from settings. Called after a save.
    pub fn rebind(&self) -> speechify_core::Result<()> {
        let tracker = Tracker::new(&self.db().settings().hotkeys)?;
        let old = lock(&self.tracker).replace(tracker);
        // The new tracker waits for held keys to be let go without reporting
        // it, so a shortcut held now must not be left down.
        for bind in old.as_ref().map(Tracker::down).unwrap_or_default() {
            self.send(Msg::Input(Event::Up(bind)));
        }
        self.binding_revision.fetch_add(1, Ordering::Release);
        Ok(())
    }
}

/// Where the bundled speech runtime lives. `SPEECHIFY_NATIVE_DIR` overrides
/// it when running from source.
fn runtime_dir(app: &AppHandle) -> PathBuf {
    // Only builds from source honour the override; an installed app always
    // loads the runtime it shipped with.
    if let Some(dir) = std::env::var_os("SPEECHIFY_NATIVE_DIR").filter(|_| cfg!(debug_assertions)) {
        return PathBuf::from(dir);
    }
    let bundled = app
        .path()
        .resource_dir()
        .map(|dir| dir.join("resources").join("native"))
        .unwrap_or_default();
    // A debug build run from source also finds the runtime in the source
    // tree. A release build only ever uses its bundled copy; if that is
    // missing, loading fails and names the bundled path.
    if bundled.exists() || !cfg!(debug_assertions) {
        return bundled;
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources")
        .join("native")
}

/// Lists compute devices in a child process (`speechify --probe-devices`),
/// so a GPU driver that crashes while registering cannot crash the app. A
/// probe that fails or hangs reports no devices, which picks the CPU models.
fn probe_devices(runtime_dir: &std::path::Path) -> Vec<speechify_core::catalog::Device> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    let Ok(exe) = std::env::current_exe() else { return vec![] };
    let spawned = Command::new(exe)
        .arg("--probe-devices")
        .arg(runtime_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = spawned else { return vec![] };
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return vec![];
            }
        }
    }
    let mut out = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_string(&mut out);
    }
    serde_json::from_str(out.trim()).unwrap_or_default()
}

pub fn start(app: AppHandle) -> Result<Arc<Shared>, Box<dyn std::error::Error>> {
    let data_dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(&data_dir)?;
    let db_path = data_dir.join("speechify.db");
    let (tx, rx) = channel();
    // The key reader is tied to the thread that creates it, so this one only
    // answers "can we read keys at all?".
    let keyboard_available = crate::diagnostics::scope("keyboard.open", || Keyboard::new().available());
    crate::diagnostics::event("keyboard.available", if keyboard_available { "yes" } else { "no" });
    let shared = Arc::new(Shared {
        tx: Mutex::new(tx),
        db: Mutex::new(Store::open(&db_path)?),
        tracker: Mutex::new(None),
        binding_revision: AtomicUsize::new(0),
        shortcuts_paused_until: Mutex::new(None),
        speech: Arc::new(Mutex::new(SpeechStatus::default())),
        teacher_running: AtomicBool::new(false),
        quitting: Arc::new(AtomicBool::new(false)),
        exiting: AtomicBool::new(false),
        stopped: (Mutex::new(false), Condvar::new()),
        wanted: AtomicUsize::new(0),
        downloads: Mutex::new(HashMap::new()),
        runtime_dir: runtime_dir(&app),
        data_dir,
        keyboard_available: AtomicBool::new(keyboard_available),
    });
    // Unusable shortcuts in a hand-edited database must not stop the app.
    let _ = shared.rebind();

    // Keyboard thread: turns key state into shortcut events.
    let keys = shared.clone();
    std::thread::spawn(move || {
        let mut keyboard = crate::diagnostics::scope("keyboard.reader.open", Keyboard::new);
        keys.keyboard_available.store(keyboard.available(), Ordering::Relaxed);
        let mut access_checked = Instant::now();
        let mut previous = Vec::new();
        let mut revision = 0;
        // The tracker times modifier-only shortcuts against this.
        let clock = Instant::now();
        let mut was_paused = false;
        loop {
            if keys.quitting.load(Ordering::Relaxed) {
                return;
            }
            if access_checked.elapsed() >= Duration::from_secs(1) {
                let available = keyboard.refresh_access();
                let previous_access = keys.keyboard_available.swap(available, Ordering::Relaxed);
                if previous_access != available {
                    crate::diagnostics::event("keyboard.access", if available { "granted" } else { "denied" });
                    previous.clear();
                    let _ = keys.rebind();
                    if !available {
                        // Revoking keyboard access must not leave a held
                        // recording running with no way to observe key-up.
                        keys.send(Msg::Input(Event::BarCancel));
                    }
                }
                access_checked = Instant::now();
            }
            if !keyboard.available() {
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }
            std::thread::sleep(Duration::from_millis(12));
            // While paused every key reads as up, which also ends a held
            // dictation. Keys still down when it ends were for the recorder.
            let paused = keys.shortcuts_paused();
            if was_paused && !paused {
                if let Some(tracker) = lock(&keys.tracker).as_mut() {
                    tracker.hold_off();
                }
                // Feed the tracker even if no key changed, so that what is
                // not held goes back up.
                keys.binding_revision.fetch_add(1, Ordering::Release);
            }
            was_paused = paused;
            let names = if paused { Vec::new() } else { keyboard.pressed() };
            let current_revision = keys.binding_revision.load(Ordering::Acquire);
            // A settling shortcut goes down after a while with no key change.
            let settling = lock(&keys.tracker).as_ref().is_some_and(Tracker::settling);
            if names == previous && current_revision == revision && !settling {
                continue;
            }
            revision = current_revision;
            previous.clone_from(&names);
            let pressed: BTreeSet<Key> = names.iter().map(|name| Key::from_name(name)).collect();
            let edges = lock(&keys.tracker)
                .as_mut()
                .map(|tracker| tracker.update(&pressed, clock.elapsed().as_millis() as u64))
                .unwrap_or_default();
            for edge in edges {
                keys.send(Msg::Input(match edge {
                    Edge::Down(bind) => Event::Down(bind),
                    Edge::Up(bind) => Event::Up(bind),
                    Edge::Abort(bind) => Event::Abort(bind),
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
            // First start: pick the models this machine runs fast.
            if store.get("settings").is_none() {
                let devices = probe_devices(&state.runtime_dir);
                if let Ok(Some(plan)) = speechify_core::catalog::first_run(&store, &devices) {
                    let _ = store.set("first_run_plan", &plan.reason);
                }
            }
            let desktop = Desktop::new(app.clone(), state.audio_dir(), state.quitting.clone());
            let models = Models::new(state.runtime_dir.clone(), state.models_dir(), state.speech.clone());
            let mut controller = Controller::new(desktop, models, store);
            let settings = controller.store.settings();
            controller.engines.preload(&settings);
            if controller.apply_retention() {
                let _ = app.emit("history-changed", ());
            }
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
                    // A new history setting takes effect at once.
                    if controller.apply_retention() {
                        let _ = app.emit("history-changed", ());
                    }
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
                            crate::diagnostics::scope("input.handle", || controller.handle(event));
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
                            crate::diagnostics::event("input.stale", "discarded");
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
                            // Dictations kept "for a day" expire even when nobody dictates.
                            if controller.apply_retention() {
                                let _ = app.emit("history-changed", ());
                            }
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
        // The controller, and with it every model, was dropped when the
        // closure above returned or unwound.
        *lock(&state.stopped.0) = true;
        state.stopped.1.notify_all();
        if state.quitting.load(Ordering::SeqCst) {
            crate::diagnostics::event("worker", "shutdown_complete");
            if !state.exiting.load(Ordering::SeqCst) {
                app.exit(0);
            }
        } else if result.is_err() {
            crate::diagnostics::event("worker", "panicked");
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
