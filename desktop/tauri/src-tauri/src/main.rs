#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]
mod audio;
#[cfg(test)]
mod behavior_tests;
mod child_owner;
mod config;
#[cfg(feature = "desktop")]
mod desktop;
#[cfg(target_os = "linux")]
mod evdev;
mod ffi;
mod installer;
mod keys;
mod learning;
#[cfg(all(feature = "desktop", target_os = "macos"))]
mod mac_clipboard;
mod models;
mod native;
#[cfg(feature = "desktop")]
mod platform;
#[cfg(feature = "desktop")]
mod replacement;
mod service;
mod store;
mod worker;
#[cfg(all(feature = "desktop", target_os = "linux"))]
mod xclipboard;
fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
fn run() -> config::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    #[cfg(all(feature = "desktop", target_os = "linux"))]
    if args.get(1).is_some_and(|s| s == "--clipboard-worker") {
        return xclipboard::worker();
    }
    if args.get(1).is_some_and(|s| s == "--native-worker") {
        if args.len() != 6 {
            return Err("native-worker requires native-dir model device threads".into());
        }
        return worker::serve(
            std::path::Path::new(&args[2]),
            std::path::Path::new(&args[3]),
            &args[4],
            args[5].parse().map_err(|_| "Invalid thread count")?,
        );
    }
    if args.get(1).is_some_and(|s| s == "fixture") {
        if args.len() < 4 {
            return Err("fixture MODEL_ID AUDIO.wav [REPEATS] [DEVICE]".into());
        }
        let root = config::root()?;
        let mut cfg = config::load(&root)?;
        cfg["model"] = serde_json::json!(args[2]);
        cfg["device"] = serde_json::json!(args.get(5).map(|x| x.as_str()).unwrap_or("cpu"));
        let native = worker::native_dir(None);
        let service = service::Service::new(root, native)?;
        *service.config.lock().unwrap() = cfg;
        for i in 0..args
            .get(4)
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(1)
        {
            let result = service.infer_path(std::path::Path::new(&args[3]), false)?;
            println!("{}", serde_json::json!({"iteration":i,"result":result}));
        }
        return Ok(());
    }
    if args.get(1).is_some_and(|s| s == "install-model") {
        let id = args.get(2).ok_or("install-model MODEL_ID")?;
        return models::install(
            &config::root()?,
            id,
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            |p| eprintln!("{p}"),
        );
    }
    if args.get(1).is_some_and(|s| s == "diagnostics") {
        println!("{}", service::diagnostics());
        return Ok(());
    }
    #[cfg(feature = "desktop")]
    return desktop::run();
    #[cfg(not(feature = "desktop"))]
    Err("This build contains only fixture tools; build default features for desktop".into())
}
