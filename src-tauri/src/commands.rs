//! The commands the UI can call. Each mirrors a function in `src/api.ts`.

use crate::desktop::clock;
use crate::shell;
use crate::worker::{Msg, Shared};
use serde::Serialize;
use speechify_core::context::{categorize, Rule, Target};
use speechify_core::format::Replacement;
use speechify_core::hotkey::Tracker;
use speechify_core::insights::{self, Insights};
use speechify_core::llm::{Chat, Rewriter};
use speechify_core::session::Event;
use speechify_core::settings::Settings;
use speechify_core::store::{Dictation, Note, Snippet, Suggestion, Word};
use speechify_core::teacher::{self, Report};
use speechify_core::{catalog, pipeline, Result};
use std::collections::hash_map::Entry;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_autostart::ManagerExt;

type App<'a> = State<'a, Arc<Shared>>;

#[tauri::command]
pub fn get_settings(shared: App) -> Settings {
    shared.db().settings()
}

#[tauri::command]
pub fn save_settings(app: AppHandle, shared: App, settings: Settings) -> Result<()> {
    // Reject unusable shortcuts before anything is stored.
    Tracker::new(&settings.hotkeys)?;
    catalog::find(&settings.speech.model)?;
    catalog::find(&settings.teacher.speech_model)?;
    let launcher = app.autolaunch();
    let launch_result = if settings.launch_at_login { launcher.enable() } else { launcher.disable() };
    launch_result.map_err(|error| format!("Could not update launch at login: {error}"))?;
    shared.db().save_settings(&settings)?;
    shared.rebind()?;
    shell::place_bar(&app, &speechify_core::app::Bar::Idle);
    shared.send(Msg::Reload);
    Ok(())
}

#[derive(Serialize)]
pub struct Status {
    version: String,
    platform: &'static str,
    wayland: bool,
    speech_ready: bool,
    speech_device: String,
    speech_error: String,
    rewrite_reachable: bool,
    rewrite_models: Vec<String>,
    microphones: Vec<String>,
}

fn chat(settings: &Settings, model: &str, seconds: u64) -> Chat {
    Chat::new(
        &settings.rewrite.url,
        &settings.rewrite.key,
        model,
        Duration::from_secs(seconds),
    )
}

#[tauri::command(async)]
pub fn get_status(shared: App) -> Status {
    let settings = shared.db().settings();
    let speech = shared.speech.lock().map(|status| status.clone()).unwrap_or_default();
    let models = chat(&settings, &settings.rewrite.model, 2).models();
    Status {
        version: crate::VERSION.to_string(),
        platform: if cfg!(target_os = "windows") {
            "windows"
        } else if cfg!(target_os = "macos") {
            "macos"
        } else {
            "linux"
        },
        wayland: !shared.keyboard_available,
        speech_ready: speech.ready,
        speech_device: speech.device,
        speech_error: speech.error,
        rewrite_reachable: models.is_ok(),
        rewrite_models: models.unwrap_or_default(),
        microphones: speechify_os::capture::microphones(),
    }
}

#[tauri::command]
pub fn history(shared: App, search: String, limit: u32, offset: u32) -> Result<Vec<Dictation>> {
    shared.db().history(&search, limit.min(500), offset)
}

#[tauri::command]
pub fn edit_dictation(shared: App, id: i64, text: String) -> Result<Vec<Replacement>> {
    let db = shared.db();
    teacher::learn_from_edit(&db, &db.settings(), id, &text, clock())
}

#[tauri::command]
pub fn delete_dictations(app: AppHandle, shared: App, ids: Option<Vec<i64>>) -> Result<()> {
    for audio in shared.db().delete_dictations(ids.as_deref())? {
        let _ = std::fs::remove_file(shared.audio_dir().join(audio));
    }
    let _ = app.emit("history-changed", ());
    Ok(())
}

#[tauri::command]
pub fn flag_dictation(shared: App, id: i64, flagged: bool) -> Result<()> {
    shared.db().set_flagged(id, flagged)
}

/// Base64 without a dependency: recordings are small.
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n =
            (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[tauri::command(async)]
pub fn dictation_audio(shared: App, id: i64) -> Result<String> {
    let audio = shared.db().dictation(id)?.audio;
    if audio.is_empty() {
        return Ok(String::new());
    }
    match std::fs::read(shared.audio_dir().join(audio)) {
        Ok(bytes) => Ok(format!("data:audio/wav;base64,{}", base64(&bytes))),
        Err(_) => Ok(String::new()),
    }
}

#[tauri::command]
pub fn copy_text(text: String) -> Result<()> {
    speechify_os::typing::copy(&text)
}

#[tauri::command]
pub fn get_insights(shared: App) -> Result<Insights> {
    Ok(insights::compute(&shared.db().all()?, clock()))
}

#[tauri::command]
pub fn dictionary(shared: App) -> Result<Vec<Word>> {
    shared.db().dictionary()
}

#[tauri::command]
pub fn add_word(shared: App, heard: String, term: String) -> Result<()> {
    shared.db().add_word(&heard, &term, "manual", clock()).map(|_| ())
}

#[tauri::command]
pub fn remove_word(shared: App, id: i64) -> Result<()> {
    shared.db().remove_word(id)
}

#[tauri::command]
pub fn suggestions(shared: App) -> Result<Vec<Suggestion>> {
    shared.db().suggestions()
}

#[tauri::command]
pub fn resolve_suggestion(shared: App, id: i64, accept: bool) -> Result<()> {
    shared.db().resolve_suggestion(id, accept, "teacher", clock())
}

#[tauri::command]
pub fn snippets(shared: App) -> Result<Vec<Snippet>> {
    shared.db().snippets()
}

#[tauri::command]
pub fn save_snippet(shared: App, id: Option<i64>, trigger: String, expansion: String) -> Result<i64> {
    shared.db().save_snippet(id, &trigger, &expansion)
}

#[tauri::command]
pub fn delete_snippet(shared: App, id: i64) -> Result<()> {
    shared.db().delete_snippet(id)
}

#[tauri::command]
pub fn notes(shared: App) -> Result<Vec<Note>> {
    shared.db().notes()
}

#[tauri::command]
pub fn save_note(shared: App, id: Option<i64>, title: String, body: String) -> Result<i64> {
    shared.db().save_note(id, &title, &body, clock())
}

#[tauri::command]
pub fn delete_note(shared: App, id: i64) -> Result<()> {
    shared.db().delete_note(id)
}

#[tauri::command]
pub fn rules(shared: App) -> Vec<Rule> {
    shared.db().rules()
}

#[tauri::command]
pub fn save_rules(shared: App, rules: Vec<Rule>) -> Result<()> {
    shared.db().save_rules(&rules)
}

#[derive(Serialize)]
pub struct SpeechModel {
    id: String,
    name: String,
    bytes: u64,
    notes: String,
    wants_gpu: bool,
    installed: bool,
    active: bool,
    teacher: bool,
}

#[tauri::command]
pub fn models(shared: App) -> Vec<SpeechModel> {
    let settings = shared.db().settings();
    let dir = shared.models_dir();
    catalog::models()
        .into_iter()
        .map(|model| SpeechModel {
            installed: catalog::installed(&dir, &model),
            active: settings.speech.model == model.id,
            teacher: settings.teacher.speech_model == model.id,
            id: model.id,
            name: model.name,
            bytes: model.bytes,
            notes: model.notes,
            wants_gpu: model.wants_gpu,
        })
        .collect()
}

#[derive(Serialize, Clone)]
struct Progress {
    id: String,
    done: u64,
    total: u64,
    finished: bool,
    error: String,
}

#[tauri::command]
pub fn download_model(app: AppHandle, shared: App, id: String) -> Result<()> {
    let model = catalog::find(&id)?;
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut downloads = shared.downloads.lock().map_err(|_| "Downloads are unavailable.")?;
        match downloads.entry(id.clone()) {
            Entry::Occupied(_) => return Ok(()),
            Entry::Vacant(entry) => {
                entry.insert(cancel.clone());
            }
        }
    }
    let shared = shared.inner().clone();
    std::thread::spawn(move || {
        let mut last = std::time::Instant::now();
        let result = catalog::download(&shared.models_dir(), &model, &mut |done, total| {
            // A few updates a second is plenty for a progress bar.
            if last.elapsed() > Duration::from_millis(200) {
                last = std::time::Instant::now();
                let _ = app.emit(
                    "model-progress",
                    Progress {
                        id: model.id.clone(),
                        done,
                        total,
                        finished: false,
                        error: String::new(),
                    },
                );
            }
            !cancel.load(Ordering::Relaxed)
        });
        if let Ok(mut downloads) = shared.downloads.lock() {
            downloads.remove(&model.id);
        }
        let _ = app.emit(
            "model-progress",
            Progress {
                id: model.id.clone(),
                done: model.bytes,
                total: model.bytes,
                finished: true,
                error: result.err().unwrap_or_default(),
            },
        );
        // The model the user is waiting for can load now.
        shared.send(Msg::Reload);
    });
    Ok(())
}

#[tauri::command]
pub fn cancel_download(shared: App, id: String) {
    if let Ok(downloads) = shared.downloads.lock() {
        if let Some(cancel) = downloads.get(&id) {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}

#[tauri::command]
pub fn delete_model(shared: App, id: String) -> Result<()> {
    let model = catalog::find(&id)?;
    let path = catalog::path(&shared.models_dir(), &model);
    if shared
        .downloads
        .lock()
        .map_err(|_| "Downloads are unavailable.")?
        .contains_key(&id)
    {
        return Err("Cancel this download and wait for it to finish before deleting the model.".into());
    }
    let _ = std::fs::remove_file(path.with_extension("part"));
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| {
            format!(
                "Could not delete {}: {e}. Is it in use? Switch models first.",
                model.name
            )
        })?;
    }
    Ok(())
}

#[tauri::command(async)]
pub fn pull_rewrite_model(app: AppHandle, shared: App, model: String) -> Result<()> {
    let settings = shared.db().settings();
    let chat = chat(&settings, &model, 10);
    // Fail now, with a clear message, if the server is not there.
    chat.models()?;
    std::thread::spawn(move || {
        let mut last = std::time::Instant::now();
        let result = chat.pull(&mut |done, total| {
            if last.elapsed() > Duration::from_millis(200) {
                last = std::time::Instant::now();
                let _ = app.emit(
                    "rewrite-progress",
                    Progress {
                        id: model.clone(),
                        done,
                        total,
                        finished: false,
                        error: String::new(),
                    },
                );
            }
        });
        let _ = app.emit(
            "rewrite-progress",
            Progress {
                id: model.clone(),
                done: 0,
                total: 0,
                finished: true,
                error: result.err().unwrap_or_default(),
            },
        );
    });
    Ok(())
}

#[derive(Serialize)]
pub struct TeacherStatus {
    running: bool,
    last_run: Option<i64>,
    report: Option<Report>,
    waiting: usize,
}

#[tauri::command]
pub fn teacher_status(shared: App) -> TeacherStatus {
    let db = shared.db();
    TeacherStatus {
        running: shared.teacher_running.load(Ordering::Relaxed),
        last_run: db.get("teacher_last_run").and_then(|value| value.parse().ok()),
        report: db
            .get("teacher_last_report")
            .and_then(|json| serde_json::from_str(&json).ok()),
        waiting: db.pending_review(100_000).map(|rows| rows.len()).unwrap_or(0),
    }
}

#[tauri::command]
pub fn run_teacher(shared: App) {
    shared.send(Msg::Teacher);
}

#[tauri::command]
pub fn export_training(shared: App) -> Result<String> {
    let lines = teacher::export(&shared.db().all()?);
    if lines.is_empty() {
        return Err("Nothing to export yet. Dictations appear here after a nightly review.".into());
    }
    let path = shared.data_dir.join("training.jsonl");
    std::fs::write(&path, lines).map_err(|e| format!("Could not write {}: {e}", path.display()))?;
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command(async)]
pub fn try_format(shared: App, text: String, app: String) -> String {
    // Formatting may wait for the language server. Do not hold the UI DB
    // mutex: the dictation worker also needs it to place the bar.
    let Ok(db) = speechify_core::store::Store::open(&shared.data_dir.join("speechify.db")) else {
        return String::new();
    };
    let settings = db.settings();
    let target = Target {
        app,
        process: String::new(),
        title: String::new(),
    };
    let category = categorize(&target, &db.rules());
    let model = chat(
        &settings,
        &settings.rewrite.model,
        settings.rewrite.timeout_ms.div_ceil(1000).max(1),
    );
    let rewriter = settings.rewrite.enabled.then_some(&model as &dyn Rewriter);
    pipeline::compose(&text, &target, category, &settings, &db, rewriter)
        .map(|composed| composed.text)
        .unwrap_or_default()
}

#[tauri::command]
pub fn bar_action(app: AppHandle, shared: App, action: String) {
    match action.as_str() {
        "click" => shared.send(Msg::Input(Event::BarClick)),
        "cancel" => shared.send(Msg::Input(Event::BarCancel)),
        "undo" => shared.send(Msg::Input(Event::Undo)),
        _ => shell::show_bar(&app, &speechify_core::app::Bar::Idle),
    }
}

#[tauri::command]
pub fn open_main(app: AppHandle, page: String) {
    shell::show_main(&app, &page);
}
