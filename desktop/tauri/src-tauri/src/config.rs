use serde_json::{json, Value};
use std::path::{Path, PathBuf};
pub type Result<T> = std::result::Result<T, String>;
pub fn defaults() -> Value {
    let mut v: Value = serde_json::from_str(include_str!("../resources/defaults.json")).unwrap();
    if cfg!(target_os = "macos") {
        v["ptt_key"] = json!("alt_r");
    }
    v
}
pub fn root() -> Result<PathBuf> {
    let base = if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
    }
    .ok_or("No user data directory")?;
    // A test override is a parent, never a direct existing Speechify root.
    let p = std::env::var_os("SPEECHIFY_CANDIDATE_PARENT")
        .map(PathBuf::from)
        .unwrap_or(base)
        .join("speechify-tauri-candidate");
    std::fs::create_dir_all(&p).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    for sub in ["audio", "models", "logs", "exports"] {
        std::fs::create_dir_all(p.join(sub)).map_err(|e| e.to_string())?;
    }
    Ok(p)
}
pub fn validate(v: &Value) -> Result<()> {
    let d = defaults();
    for (k, val) in d.as_object().unwrap() {
        let x = v.get(k).ok_or(format!("Missing setting {k}"))?;
        if std::mem::discriminant(x) != std::mem::discriminant(val) {
            return Err(format!("Invalid type for {k}"));
        }
    }
    for (key, min, max) in [
        ("min_hold_ms", 0., 5000.),
        ("beam_size", 1., 32.),
        ("onnx_threads", 0., 128.),
        ("autocorrect_after", 1., 100.),
        ("teacher_min_confidence", 0., 1.),
        ("keep_versions", 1., 100.),
        ("idle_minutes", 0.1, 1440.),
        ("retrain_after_labels", 1., 100000.),
        ("retrain_after_days", 0.1, 3650.),
        ("max_regression_wer", 0., 100.),
    ] {
        let x = v[key].as_f64().ok_or(format!("{key} must be numeric"))?;
        if !x.is_finite() || x < min || x > max {
            return Err(format!("{key} must be {min}–{max}"));
        }
    }
    for k in [
        "min_hold_ms",
        "beam_size",
        "onnx_threads",
        "autocorrect_after",
        "keep_versions",
        "retrain_after_labels",
    ] {
        if v[k].as_u64().is_none() {
            return Err(format!("{k} must be a nonnegative whole number"));
        }
    }
    for (k, allowed) in [
        ("device", vec!["auto", "cpu", "cuda", "vulkan", "metal"]),
        ("theme", vec!["system", "dark", "light"]),
        (
            "compute_type",
            vec!["float16", "float32", "int8", "int8_float16", "auto"],
        ),
    ] {
        if !allowed.contains(&v[k].as_str().unwrap_or("")) {
            return Err(format!("Invalid {k}"));
        }
    }
    let mut combos = std::collections::HashSet::new();
    for k in [
        "ptt_key",
        "correct_hotkey",
        "history_hotkey",
        "models_hotkey",
    ] {
        let s = v[k].as_str().unwrap_or("");
        let normalized = crate::keys::parse(s)?;
        #[cfg(target_os = "macos")]
        if normalized
            .split('+')
            .any(|key| ["scroll_lock", "f21", "f22", "f23", "f24"].contains(&key))
        {
            return Err(
                "macOS does not expose Scroll Lock or F21–F24 to the global shortcut poller".into(),
            );
        }
        if !combos.insert(normalized) {
            return Err("Shortcut conflicts with another action".into());
        }
    }
    if !v["llm_url"].as_str().unwrap().is_empty() {
        let url = reqwest::Url::parse(v["llm_url"].as_str().unwrap()).map_err(|e| e.to_string())?;
        if !["http", "https"].contains(&url.scheme()) {
            return Err("LLM endpoint must use HTTP or HTTPS".into());
        }
    }
    Ok(())
}
pub fn load(root: &Path) -> Result<Value> {
    let path = root.join("config.toml");
    let mut d = defaults();
    if path.exists() {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let v: toml::Value = toml::from_str(&text).map_err(|e| e.to_string())?;
        let obj = serde_json::to_value(v).map_err(|e| e.to_string())?;
        for (k, v) in obj.as_object().ok_or("Invalid config")? {
            d[k] = v.clone();
        }
    }
    validate(&d)?;
    Ok(d)
}
pub fn save(root: &Path, v: &Value) -> Result<()> {
    validate(v)?;
    let text = toml::to_string_pretty(v).map_err(|e| e.to_string())?;
    atomic(&root.join("config.toml"), text.as_bytes())
}
pub fn atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let partial = path.with_extension("tmp");
    let mut f = std::fs::File::create(&partial).map_err(|e| e.to_string())?;
    f.write_all(bytes)
        .and_then(|_| f.sync_all())
        .map_err(|e| e.to_string())?;
    std::fs::rename(partial, path).map_err(|e| e.to_string())
}
pub fn owned(root: &Path, name: &str) -> Result<PathBuf> {
    if name.is_empty() || Path::new(name).components().count() != 1 || name == ".." || name == "." {
        return Err("Unsafe candidate filename".into());
    }
    let p = root.join(name);
    if p.exists()
        && !p
            .canonicalize()
            .map_err(|e| e.to_string())?
            .starts_with(root.canonicalize().map_err(|e| e.to_string())?)
    {
        return Err("Path leaves candidate directory".into());
    }
    Ok(p)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn config_validation() {
        let mut v = defaults();
        assert!(validate(&v).is_ok());
        v["min_hold_ms"] = json!(-1);
        assert!(validate(&v).is_err());
    }
    #[test]
    fn traversal() {
        assert!(owned(Path::new("/tmp"), "../../x").is_err());
    }
}

/// Candidate-specific advisory lock; fixture commands never touch the installed app lock.
pub fn lock(root: &Path) -> Result<std::fs::File> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("service.lock"))
        .map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err("Another process owns this candidate data directory. Use a separate SPEECHIFY_CANDIDATE_PARENT for fixture tests.".into());
        }
    }
    Ok(file)
}
