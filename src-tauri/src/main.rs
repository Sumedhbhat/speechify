//! Speechify's desktop shell: windows, tray and threads around the core.
#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

mod commands;
mod desktop;
mod diagnostics;
mod models;
mod shell;
mod updates;
mod worker;

use speechify_core::session::Event;
use std::sync::Arc;
use tauri::{Manager, RunEvent, WindowEvent};

/// The release build stamps its full version (including nightly numbers).
pub const VERSION: &str = match option_env!("SPEECHIFY_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

fn main() {
    if std::env::args().any(|arg| arg == "--version") {
        println!("speechify {VERSION}");
        return;
    }
    let args: Vec<String> = std::env::args().collect();
    // A short-lived child run by the first start to list compute devices.
    // Registering GPU backends runs driver code that can crash; doing it here
    // keeps a broken driver from taking down the app itself.
    if let Some(at) = args.iter().position(|arg| arg == "--probe-devices") {
        let dir = std::path::PathBuf::from(args.get(at + 1).cloned().unwrap_or_default());
        let devices = speechify_core::native::devices(&dir).unwrap_or_default();
        println!("{}", serde_json::to_string(&devices).unwrap_or_else(|_| "[]".into()));
        return;
    }
    let toggle = args.iter().any(|arg| arg == "--toggle");
    let cancel = args.iter().any(|arg| arg == "--cancel");
    let background = args.iter().any(|arg| arg == "--background") || toggle || cancel;
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _| {
            // A second launch talks to the running app: `speechify --toggle`
            // starts or stops dictation (for system shortcuts on Wayland).
            let Some(shared) = app.try_state::<Arc<worker::Shared>>() else { return };
            if args.iter().any(|arg| arg == "--toggle") {
                shared.send(worker::Msg::Input(Event::BarClick));
            } else if args.iter().any(|arg| arg == "--update") {
                // `speechify --update` installs the newest release, if any.
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = updates::install(&app).await;
                });
            } else if args.iter().any(|arg| arg == "--cancel") {
                shared.send(worker::Msg::Input(Event::BarCancel));
            } else if !args.iter().any(|arg| arg == "--background") {
                shell::show_main(app, "home");
            }
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--background"]),
        ))
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::save_settings,
            commands::pause_shortcuts,
            commands::get_status,
            commands::rewrite_status,
            commands::history,
            commands::edit_dictation,
            commands::delete_dictations,
            commands::flag_dictation,
            commands::dictation_audio,
            commands::copy_text,
            commands::get_insights,
            commands::dictionary,
            commands::add_word,
            commands::remove_word,
            commands::suggestions,
            commands::resolve_suggestion,
            commands::snippets,
            commands::save_snippet,
            commands::delete_snippet,
            commands::notes,
            commands::save_note,
            commands::delete_note,
            commands::rules,
            commands::save_rules,
            commands::models,
            commands::download_model,
            commands::cancel_download,
            commands::delete_model,
            commands::pull_rewrite_model,
            commands::teacher_status,
            commands::run_teacher,
            commands::export_training,
            commands::export_diagnostics,
            commands::try_format,
            commands::bar_action,
            commands::bar_hit,
            commands::open_main,
            commands::check_update,
            commands::install_update,
        ])
        .setup(move |app| {
            match app.path().app_log_dir() {
                Ok(dir) => {
                    if let Err(error) = diagnostics::init(&dir) {
                        eprintln!("Could not start Speechify diagnostics: {error}");
                    }
                }
                Err(error) => eprintln!("Could not find Speechify diagnostics folder: {error}"),
            }
            diagnostics::event("app.version", VERSION);
            let shared = worker::start(app.handle().clone())?;
            app.manage(shared.clone());
            #[cfg(target_os = "macos")]
            shell::build_app_menu(app.handle())?;
            shell::build_tray(app.handle())?;
            shell::track_bar(app.handle());
            #[cfg(target_os = "macos")]
            shell::bar_never_activates(app.handle());
            shell::place_bar(app.handle(), &speechify_core::app::Bar::Idle);
            if toggle {
                shared.send(worker::Msg::StartupToggle);
            }
            // Started at login, the app stays in the tray; started by hand, it opens.
            if !background {
                shell::show_main(app.handle(), "home");
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the main window keeps dictation running in the tray.
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("Speechify could not start")
        .run(|app, event| {
            // The Dock icon stays visible on a Mac; clicking it brings the
            // window back after it was closed to the tray.
            #[cfg(target_os = "macos")]
            if let RunEvent::Reopen { .. } = event {
                shell::show_main(app, "home");
                return;
            }
            // Every way out (Quit, Cmd+Q, Dock › Quit, logout, an installer's
            // quit, an update's restart) passes here before the process exits.
            if let RunEvent::Exit = event {
                if let Some(shared) = app.try_state::<Arc<worker::Shared>>() {
                    // Usually the worker is already idle and this returns at
                    // once; a piece mid-recognition can hold the model longer.
                    shared.release_for_exit(models::RELEASE_WAIT + std::time::Duration::from_secs(5));
                }
                return;
            }
            if let RunEvent::ExitRequested { api, code, .. } = event {
                diagnostics::event(
                    "app.exit_requested",
                    if code.is_some() { "explicit" } else { "implicit" },
                );
                // Only an explicit Quit (which passes a code) ends the app.
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}
