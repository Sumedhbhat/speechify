//! Windows and tray: where the bar sits and what the tray menu does.

use crate::worker::{Msg, Shared};
use speechify_core::app::Bar;
use speechify_core::hotkey::Bind;
use speechify_core::session::Event;
use std::sync::Arc;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager};

/// Gap between the bar and the bottom of the usable screen area.
const MARGIN: f64 = 14.0;

/// The bar window is only as large as what it shows, so it never blocks
/// clicks on the apps underneath.
fn bar_size(bar: &Bar) -> (f64, f64) {
    match bar {
        Bar::Idle => (220.0, 48.0),
        Bar::Listening { .. } | Bar::Processing | Bar::Done { .. } => (260.0, 64.0),
        Bar::Notice { .. } => (440.0, 132.0),
        Bar::Card { .. } => (440.0, 300.0),
    }
}

// Tao clears its initial DONT_FOCUS marker after creation, so show() later
// uses SW_SHOW. Keep visibility native on Windows and explicitly avoid activation.
#[cfg(windows)]
fn bar_visible(window: &tauri::WebviewWindow, visible: bool) {
    let window = window.clone();
    let app = window.app_handle().clone();
    let _ = app.run_on_main_thread(move || {
        #[link(name = "user32")]
        extern "system" {
            fn ShowWindow(window: *mut std::ffi::c_void, command: i32) -> i32;
        }
        if let Ok(handle) = window.hwnd() {
            unsafe {
                ShowWindow(
                    handle.0,
                    if visible {
                        4 /* SW_SHOWNOACTIVATE */
                    } else {
                        0 /* SW_HIDE */
                    },
                );
            }
        }
    });
}

#[cfg(not(windows))]
fn bar_visible(window: &tauri::WebviewWindow, visible: bool) {
    let _ = if visible { window.show() } else { window.hide() };
}

/// Sizes the bar for its state and centres it at the bottom of the screen.
pub fn place_bar(app: &AppHandle, bar: &Bar) {
    let Some(window) = app.get_webview_window("bar") else {
        return;
    };
    let always = app
        .try_state::<Arc<Shared>>()
        .is_none_or(|shared| shared.db().settings().show_bar_always);
    if *bar == Bar::Idle && !always {
        bar_visible(&window, false);
        return;
    }
    let (width, height) = bar_size(bar);
    let _ = window.set_size(LogicalSize::new(width, height));
    if let Ok(Some(monitor)) = window.current_monitor().or_else(|_| window.primary_monitor()) {
        let scale = monitor.scale_factor();
        let area = monitor.work_area();
        let (left, top) = (area.position.x as f64 / scale, area.position.y as f64 / scale);
        let (area_width, area_height) = (area.size.width as f64 / scale, area.size.height as f64 / scale);
        let _ = window.set_position(LogicalPosition::new(
            left + (area_width - width) / 2.0,
            top + area_height - height - MARGIN,
        ));
    }
    bar_visible(&window, true);
}

pub fn show_bar(app: &AppHandle, bar: &Bar) {
    place_bar(app, bar);
    let _ = app.emit("bar", bar);
}

pub fn show_main(app: &AppHandle, page: &str) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        let _ = app.emit_to("main", "navigate", page);
    }
}

pub fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let item = |id: &str, label: &str| MenuItem::with_id(app, id, label, true, None::<&str>);
    let menu = Menu::with_items(
        app,
        &[
            &item("open", "Open Speechify")?,
            &item("toggle", "Start or stop dictation")?,
            &PredefinedMenuItem::separator(app)?,
            &item("paste", "Paste last dictation")?,
            &item("copy", "Copy last dictation")?,
            &PredefinedMenuItem::separator(app)?,
            &item("settings", "Settings")?,
            &item("quit", "Quit Speechify")?,
        ],
    )?;
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("Speechify")
        .menu(&menu)
        .show_menu_on_left_click(true);
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.on_menu_event(|app, event| {
        let shared = app.state::<Arc<Shared>>();
        match event.id().as_ref() {
            "open" => show_main(app, "home"),
            "settings" => show_main(app, "settings"),
            "toggle" => shared.send(Msg::Input(Event::BarClick)),
            "paste" => {
                // The menu has closed and no keys are held, so paste right away.
                shared.send(Msg::Input(Event::Down(Bind::PasteLast)));
                shared.send(Msg::Input(Event::Released));
            }
            "copy" => shared.send(Msg::Input(Event::Down(Bind::CopyLast))),
            "quit" => shared.quit(app),
            _ => {}
        }
    })
    .build(app)?;
    Ok(())
}
