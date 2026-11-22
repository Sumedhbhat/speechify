//! Windows and tray: where the bar sits and what the tray menu does.

use crate::worker::{Msg, Shared};
use speechify_core::app::Bar;
use speechify_core::hotkey::Bind;
use speechify_core::pointer::{self, Seen};
use speechify_core::session::Event;
use std::sync::{Arc, Mutex};
use std::time::Duration;
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
    // A hidden window may report no current monitor; use the primary one then.
    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten());
    if let Some(monitor) = monitor {
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

#[derive(Default)]
pub struct BarHit(Mutex<Vec<pointer::Area>>);

impl BarHit {
    pub fn set(&self, areas: Vec<pointer::Area>) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = areas;
    }

    fn seen(&self, pointer: (f64, f64), origin: (f64, f64), scale: f64) -> Seen {
        let areas = self.0.lock().unwrap_or_else(|e| e.into_inner());
        Seen::Pointer {
            clickable: pointer::over(&areas, pointer, origin, scale),
            on_pill: pointer::over(&areas[..areas.len().min(1)], pointer, origin, scale),
        }
    }
}

const TRACK_EVERY: Duration = Duration::from_millis(40);
const TRACK_HIDDEN: Duration = Duration::from_millis(250);

#[cfg(windows)]
fn bar_pass_through(window: &tauri::WebviewWindow, pass: bool) {
    let window = window.clone();
    let app = window.app_handle().clone();
    let _ = app.run_on_main_thread(move || {
        #[link(name = "user32")]
        extern "system" {
            fn GetWindowLongPtrW(window: *mut std::ffi::c_void, index: i32) -> isize;
            fn SetWindowLongPtrW(window: *mut std::ffi::c_void, index: i32, value: isize) -> isize;
        }
        const GWL_EXSTYLE: i32 = -20;
        const WS_EX_TRANSPARENT: isize = 0x20;
        const WS_EX_LAYERED: isize = 0x80000;
        const BITS: isize = WS_EX_TRANSPARENT | WS_EX_LAYERED;
        if let Ok(handle) = window.hwnd() {
            unsafe {
                let style = GetWindowLongPtrW(handle.0, GWL_EXSTYLE);
                let style = if pass { style | BITS } else { style & !BITS };
                SetWindowLongPtrW(handle.0, GWL_EXSTYLE, style);
            }
        }
    });
}

#[cfg(not(windows))]
fn bar_pass_through(window: &tauri::WebviewWindow, pass: bool) {
    let _ = window.set_ignore_cursor_events(pass);
}

pub fn track_bar(app: &AppHandle) {
    if speechify_os::is_wayland() {
        return;
    }
    app.manage(BarHit::default());
    let app = app.clone();
    let _ = std::thread::Builder::new().name("bar-pointer".into()).spawn(move || {
        let mut tracker = pointer::Tracker::default();
        let mut wait = TRACK_EVERY;
        loop {
            std::thread::sleep(wait);
            let Some(window) = app.get_webview_window("bar") else {
                continue;
            };
            let seen = if !window.is_visible().unwrap_or(false) {
                Seen::Hidden
            } else if let (Ok(cursor), Ok(origin), Ok(scale)) =
                (app.cursor_position(), window.outer_position(), window.scale_factor())
            {
                let origin = (origin.x as f64, origin.y as f64);
                app.state::<BarHit>().seen((cursor.x, cursor.y), origin, scale)
            } else {
                Seen::Unknown
            };
            wait = if seen == Seen::Hidden { TRACK_HIDDEN } else { TRACK_EVERY };
            if let Some(change) = tracker.update(seen) {
                bar_pass_through(&window, change.pass_through);
                if let Some(hover) = change.hover {
                    let _ = app.emit_to("bar", "bar-hover", hover);
                }
            }
        }
    });
}

#[cfg(target_os = "macos")]
pub fn bar_never_activates(app: &AppHandle) {
    use std::ffi::{c_char, c_void};
    #[link(name = "objc")]
    extern "C" {
        fn sel_registerName(name: *const c_char) -> *const c_void;
        fn objc_msgSend();
    }
    let Some(window) = app.get_webview_window("bar") else {
        return;
    };
    let target = window.clone();
    let _ = window.run_on_main_thread(move || {
        let Ok(ns_window) = target.ns_window() else { return };
        unsafe {
            type Responds = unsafe extern "C" fn(*mut c_void, *const c_void, *const c_void) -> i8;
            type SetBool = unsafe extern "C" fn(*mut c_void, *const c_void, i8);
            let send = objc_msgSend as unsafe extern "C" fn();
            let responds = std::mem::transmute::<unsafe extern "C" fn(), Responds>(send);
            let set = std::mem::transmute::<unsafe extern "C" fn(), SetBool>(send);
            let check = sel_registerName(c"respondsToSelector:".as_ptr());
            let prevents = sel_registerName(c"_setPreventsActivation:".as_ptr());
            if responds(ns_window, check, prevents) != 0 {
                set(ns_window, prevents, 1);
            }
        }
    });
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
    let update = item("update", "Check for Updates…")?;
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
    if crate::updates::SUPPORTED {
        // Just above "Quit Speechify".
        menu.insert(&update, 7)?;
        crate::updates::watch(app, update);
    }
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
            // The Settings page checks for updates and offers to install.
            "settings" | "update" => show_main(app, "settings"),
            "toggle" => shared.send(Msg::Input(Event::BarClick)),
            "paste" => {
                // The menu has closed and no keys are held, so paste right away.
                shared.send(Msg::Input(Event::Down(Bind::PasteLast)));
                shared.send(Msg::Input(Event::Released));
            }
            "copy" => {
                shared.send(Msg::Input(Event::Down(Bind::CopyLast)));
                shared.send(Msg::Input(Event::Released));
            }
            "quit" => shared.quit(app),
            _ => {}
        }
    })
    .build(app)?;
    Ok(())
}

/// Cocoa's predefined Quit calls terminate: directly, bypassing worker cleanup.
/// Keep the default menus but route Cmd+Q through the same path as tray Quit.
#[cfg(target_os = "macos")]
pub fn build_app_menu(app: &AppHandle) -> tauri::Result<()> {
    let menu = Menu::default(app)?;
    if let Some(tauri::menu::MenuItemKind::Submenu(application)) = menu.items()?.first() {
        let items = application.items()?;
        application.remove_at(items.len() - 1)?;
        application.append(&MenuItem::with_id(
            app,
            "app-quit",
            "Quit Speechify",
            true,
            Some("Cmd+Q"),
        )?)?;
    }
    app.set_menu(menu)?;
    app.on_menu_event(|app, event| {
        if event.id().as_ref() == "app-quit" {
            crate::diagnostics::event("app.quit", "menu");
            app.state::<Arc<Shared>>().quit(app);
        }
    });
    Ok(())
}
