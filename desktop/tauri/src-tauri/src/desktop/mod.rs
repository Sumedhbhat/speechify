//! Tauri composition root: windows, tray and application lifecycle.
mod autostart;
mod commands;
pub(crate) mod runtime;

use crate::{audio::capture::Capture, config::Result, service::Service};
use std::sync::Arc;
use tauri::{
    menu::{Menu, MenuItem},
    Emitter, Manager,
};

#[cfg(test)]
#[path = "../../tests/unit/desktop.rs"]
mod behavior;
#[cfg(test)]
#[path = "../../tests/unit/commands.rs"]
mod command_tests;

fn show(app: &tauri::AppHandle, page: &str) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
        let _ = w.emit("navigate", page);
    }
}
fn runtime(app: tauri::AppHandle, service: Arc<Service>, indicator: tauri::WebviewWindow) {
    runtime::start(
        service,
        move |page| show(&app, page),
        move |visible| {
            if visible {
                let _ = indicator.show();
            } else {
                let _ = indicator.hide();
            }
        },
        Capture::open,
    );
}
pub fn run() -> Result<()> {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            show(app, "Dictation")
        }))
        .invoke_handler(tauri::generate_handler![commands::request])
        .setup(|app| {
            let service = Service::new(
                crate::config::root().map_err(std::io::Error::other)?,
                crate::worker::native_dir(Some(app.path().resource_dir()?)),
            )
            .map_err(std::io::Error::other)?;
            app.manage(service.clone());
            let icon = tauri::image::Image::new_owned(
                {
                    let mut bytes = vec![0u8; 32 * 32 * 4];
                    for y in 0..32 {
                        for x in 0..32 {
                            let i = (y * 32 + x) * 4;
                            let h = if x % 5 < 3 { 6 + (x * 7) % 19 } else { 0 };
                            if y >= 16 - h / 2 && y <= 16 + h / 2 && h > 0 {
                                bytes[i..i + 4].copy_from_slice(&[141, 183, 255, 255]);
                            }
                        }
                    }
                    bytes
                },
                32,
                32,
            );
            let menu = Menu::new(app)?;
            for (id, text) in [
                ("settings", "Settings"),
                ("history", "History"),
                ("models", "Models"),
                ("logs", "Log location"),
                ("quit", "Quit candidate"),
            ] {
                let item = MenuItem::with_id(app, id, text, true, None::<&str>)?;
                menu.append(&item)?;
            }
            tauri::tray::TrayIconBuilder::new()
                .icon(icon)
                .tooltip("Speechify Tauri Candidate")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "quit" => {
                        app.state::<Arc<Service>>().shutdown();
                        app.exit(0);
                    }
                    "history" => show(app, "History"),
                    "models" => show(app, "Models"),
                    "logs" => show(app, "Settings"),
                    _ => show(app, "Settings"),
                })
                .build(app)?;
            let indicator = tauri::WebviewWindowBuilder::new(
                app,
                "indicator",
                tauri::WebviewUrl::App("index.html?overlay".into()),
            )
            .title("Speechify recording status")
            .inner_size(280., 68.)
            .decorations(false)
            .resizable(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .focused(false)
            .focusable(false)
            .visible(false)
            .build()?;
            runtime(app.handle().clone(), service.clone(), indicator);
            let _ = crate::learning::scheduler(service.clone());
            std::thread::spawn(move || {
                if let Err(e) = service.preload() {
                    service.error(e);
                }
            });
            Ok(())
        })
        .on_window_event(|w, e| {
            if w.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = e {
                    api.prevent_close();
                    let _ = w.hide();
                }
            }
        });
    let app = builder
        .build(tauri::generate_context!())
        .map_err(crate::config::err)?;
    app.run(|app, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            app.state::<Arc<Service>>().shutdown();
        }
    });
    Ok(())
}
