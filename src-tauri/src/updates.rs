//! In-app updates on macOS.
//!
//! The release workflow publishes a signed `.app.tar.gz` and a small feed
//! (`stable.json` or `nightly.json`) that `plugins.updater` in
//! `tauri.conf.json` points at. Installing replaces the bundle in place and
//! restarts it. Every release is signed with the same certificate, so macOS
//! keeps the Accessibility permission across updates. See docs/releases.md.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::menu::MenuItem;
use tauri::{AppHandle, Wry};
use tauri_plugin_updater::{Update, UpdaterExt};

/// Only the macOS build publishes updates.
pub const SUPPORTED: bool = cfg!(target_os = "macos");

/// The first background check waits for startup to settle.
const FIRST_CHECK: Duration = Duration::from_secs(60);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// The plugin sets none, and a stalled connection would otherwise leave a
/// check or a download (and `INSTALLING`) stuck until restart. It covers a
/// whole request, so it must also fit the ~10 MB download on a slow link.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// Asks the feed for a release newer than this one.
pub async fn find(app: &AppHandle) -> Result<Option<Update>, String> {
    let result = async { app.updater_builder().timeout(REQUEST_TIMEOUT).build()?.check().await }.await;
    crate::diagnostics::event(
        "update.check",
        match &result {
            Ok(Some(_)) => "available",
            Ok(None) => "current",
            Err(_) => "error",
        },
    );
    result.map_err(|error| format!("Could not check for updates: {error}"))
}

/// Set while an update downloads, so a second request cannot replace the app
/// at the same time (Settings reopened mid-install, the tray, `--update`).
static INSTALLING: AtomicBool = AtomicBool::new(false);

/// Downloads the newest release, replaces this app with it and restarts.
pub async fn install(app: &AppHandle) -> Result<(), String> {
    if INSTALLING.swap(true, Ordering::SeqCst) {
        return Err("An update is already installing.".into());
    }
    let result = replace_and_restart(app).await;
    INSTALLING.store(false, Ordering::SeqCst);
    result
}

/// Returns only on failure: success restarts the app.
async fn replace_and_restart(app: &AppHandle) -> Result<(), String> {
    let mut update = find(app).await?.ok_or("Speechify is already up to date.")?;
    // The builder's timeout covers only the check; the download gets none.
    update.timeout = Some(REQUEST_TIMEOUT);
    let result = update.download_and_install(|_, _| {}, || {}).await;
    crate::diagnostics::event("update.install", if result.is_ok() { "ok" } else { "error" });
    result.map_err(|error| format!("Could not install the update: {error}"))?;
    app.restart()
}

/// Checks now and then in the background and renames the tray item when a
/// newer release is out. Installing is always the user's choice.
pub fn watch(app: &AppHandle, item: MenuItem<Wry>) {
    if !SUPPORTED {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(FIRST_CHECK);
        loop {
            if let Ok(Some(update)) = tauri::async_runtime::block_on(find(&app)) {
                let _ = item.set_text(format!("Update to {}…", update.version));
            }
            std::thread::sleep(CHECK_EVERY);
        }
    });
}
