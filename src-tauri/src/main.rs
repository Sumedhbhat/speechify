//! Speechify's desktop shell: windows, tray and threads around the core.
#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

mod commands;
mod desktop;
mod models;
mod shell;
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
            } else if args.iter().any(|arg| arg == "--cancel") {
                shared.send(worker::Msg::Input(Event::BarCancel));
            } else if !args.iter().any(|arg| arg == "--background") {
                shell::show_main(app, "home");
            }
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--background"]),
        ))
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::save_settings,
            commands::get_status,
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
            commands::try_format,
            commands::bar_action,
            commands::open_main,
        ])
        .setup(move |app| {
            let shared = worker::start(app.handle().clone())?;
            app.manage(shared.clone());
            shell::build_tray(app.handle())?;
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
            #[cfg(not(target_os = "macos"))]
            let _ = app;
            if let RunEvent::ExitRequested { api, code, .. } = event {
                // Only an explicit Quit (which passes a code) ends the app.
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}
