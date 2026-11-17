//! Small, synchronous diagnostics: the last completed write survives a native abort.
//! Only fixed stage names and outcomes belong here, never user content or errors.
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const LIMIT: u64 = 1024 * 1024;
static LOG: OnceLock<Mutex<Log>> = OnceLock::new();

struct Log {
    path: PathBuf,
    file: Option<File>,
    bytes: u64,
}

fn open(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

impl Log {
    fn new(path: PathBuf) -> io::Result<Self> {
        let file = open(&path)?;
        let bytes = file.metadata()?.len();
        Ok(Self {
            path,
            file: Some(file),
            bytes,
        })
    }

    fn write(&mut self, line: &str) -> io::Result<()> {
        if self.bytes + line.len() as u64 > LIMIT {
            self.file.take(); // Windows requires closing before renaming.
            let previous = self.path.with_extension("previous.log");
            match std::fs::remove_file(&previous) {
                Ok(()) => (),
                Err(error) if error.kind() == io::ErrorKind::NotFound => (),
                Err(error) => return Err(error),
            }
            std::fs::rename(&self.path, previous)?;
            self.bytes = 0;
        }
        if self.file.is_none() {
            self.file = Some(open(&self.path)?);
        }
        let file = self.file.as_mut().expect("opened above");
        file.write_all(line.as_bytes())?;
        file.flush()?;
        self.bytes += line.len() as u64;
        Ok(())
    }
}

pub fn init(dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let log = Log::new(dir.join("diagnostics.log"))?;
    let _ = LOG.set(Mutex::new(log));
    event("app.start", env!("CARGO_PKG_VERSION"));
    event(
        "app.platform",
        &format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
    );
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Panic payloads can contain user data. Keep only the source location.
        let location = info
            .location()
            .map(|at| {
                format!(
                    "{}:{}",
                    Path::new(at.file()).file_name().unwrap_or_default().to_string_lossy(),
                    at.line()
                )
            })
            .unwrap_or_default();
        // Never block panic handling behind an in-progress filesystem write.
        if let Some(log) = LOG.get() {
            if let Ok(mut log) = log.try_lock() {
                let _ = log.write(&line("panic", &location));
            }
        }
        previous(info);
    }));
    Ok(())
}

pub fn event(stage: &str, outcome: &str) {
    let Some(log) = LOG.get() else { return };
    let failed = if let Ok(mut log) = log.lock() { log.write(&line(stage, outcome)).is_err() } else { true };
    if failed {
        // stderr may itself be unavailable. Reporting must never panic.
        let _ = writeln!(io::stderr(), "Speechify could not write diagnostics");
    }
}

fn line(stage: &str, outcome: &str) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let thread = std::thread::current().id();
    format!(
        "{now} pid={} thread={thread:?} stage={stage} {outcome}\n",
        std::process::id()
    )
}

pub fn scope<T>(stage: &'static str, action: impl FnOnce() -> T) -> T {
    let began = Instant::now();
    event(stage, "begin");
    let result = action();
    event(stage, &format!("end elapsed_ms={}", began.elapsed().as_millis()));
    result
}

pub fn attempt<T, E>(stage: &'static str, action: impl FnOnce() -> Result<T, E>) -> Result<T, E> {
    scope(stage, || {
        let result = action();
        event(stage, if result.is_ok() { "ok" } else { "error" });
        result
    })
}

/// Only these two application-owned files are exported. Reads are capped even
/// if a file was replaced or grew beyond the logger's rotation limit.
fn read_log(path: &Path) -> serde_json::Value {
    let mut bytes = Vec::new();
    match File::open(path).and_then(|file| file.take(LIMIT + 1).read_to_end(&mut bytes)) {
        Ok(_) => serde_json::json!({
            "status": if bytes.len() as u64 > LIMIT { "truncated" } else { "ok" },
            "content": String::from_utf8_lossy(&bytes[..bytes.len().min(LIMIT as usize)]),
        }),
        Err(error) => serde_json::json!({
            "status": if error.kind() == io::ErrorKind::NotFound { "missing" } else { "unreadable" },
        }),
    }
}

/// Extract native fault evidence without exporting machine identifiers, paths,
/// register values, application-specific messages or the raw crash report.
fn summarize_crash(bytes: &[u8]) -> Option<serde_json::Value> {
    let split = bytes.iter().position(|byte| *byte == b'\n')?;
    let report: serde_json::Value = serde_json::from_slice(&bytes[split + 1..]).ok()?;
    let text = |value: &serde_json::Value| value.as_str().unwrap_or_default().chars().take(256).collect::<String>();
    let frames: Vec<_> = report["threads"]
        .as_array()?
        .iter()
        .filter(|thread| thread["triggered"] == true)
        .flat_map(|thread| thread["frames"].as_array().into_iter().flatten())
        .take(64)
        .map(|frame| {
            let image = frame["imageIndex"]
                .as_u64()
                .and_then(|index| report["usedImages"].get(index as usize));
            serde_json::json!({
                "symbol": text(&frame["symbol"]),
                "library": image.map(|image| text(&image["name"])).unwrap_or_default(),
                "image_offset": frame["imageOffset"].as_u64(),
            })
        })
        .collect();
    Some(serde_json::json!({
        "captured_at": text(&report["captureTime"]),
        "pid": report["pid"].as_u64(),
        "app_version": text(&report["bundleInfo"]["CFBundleShortVersionString"]),
        "signal": text(&report["exception"]["signal"]),
        "exception": text(&report["exception"]["type"]),
        "frames": frames,
    }))
}

fn crash_summaries(dir: Option<&Path>) -> Vec<serde_json::Value> {
    let Some(dir) = dir else { return vec![] };
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut paths: Vec<_> = entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("speechify-") && name.ends_with(".ips"))
        })
        .map(|entry| entry.path())
        .collect();
    paths.sort();
    paths
        .into_iter()
        .rev()
        .take(3)
        .filter_map(|path| {
            let mut bytes = Vec::new();
            File::open(path)
                .ok()?
                .take(2 * LIMIT + 1)
                .read_to_end(&mut bytes)
                .ok()?;
            if bytes.len() as u64 > 2 * LIMIT {
                return None;
            }
            summarize_crash(&bytes)
        })
        .collect()
}

/// Save a user-requested support bundle without settings, audio or transcripts.
pub fn export(log_dir: &Path, downloads: &Path, version: &str, crash_dir: Option<&Path>) -> io::Result<PathBuf> {
    // Match the writer's mutex so rotation cannot split a snapshot across files.
    let guard = LOG.get().map(|log| log.lock());
    let logs = serde_json::json!({
        "diagnostics.previous.log": read_log(&log_dir.join("diagnostics.previous.log")),
        "diagnostics.log": read_log(&log_dir.join("diagnostics.log")),
    });
    drop(guard);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let bundle = serde_json::json!({
        "schema_version": 1,
        "app_version": version,
        "platform": std::env::consts::OS,
        "architecture": std::env::consts::ARCH,
        "exported_at_unix_ms": stamp / 1_000_000,
        "logs": logs,
        "native_crashes": crash_summaries(crash_dir),
        "privacy": "Contains application stage logs and sanitized native crash stacks. No audio, transcripts, clipboard, settings, credentials, machine identifiers or raw macOS crash reports.",
    });
    let path = downloads.join(format!("speechify-diagnostics-{stamp}.json"));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    file.write_all(serde_json::to_string_pretty(&bundle)?.as_bytes())?;
    file.flush()?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn support_export_is_bounded_and_excludes_private_files_and_crash_fields() {
        let dir = std::env::temp_dir().join(format!("speechify-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("diagnostics.log"), "typing.paste begin\n").unwrap();
        std::fs::write(dir.join("speechify.db"), "private transcript and key").unwrap();
        let raw = serde_json::json!({
            "captureTime": "2026-11-17", "pid": 42,
            "exception": {"signal": "SIGTRAP", "type": "EXC_BREAKPOINT"},
            "userID": "private-user", "procPath": "/Users/private-path", "asi": "private-message",
            "usedImages": [{"name": "HIToolbox", "path": "/private/library/path"}],
            "threads": [{"triggered": true, "threadState": "private-registers",
                "frames": [{"symbol": "TSMGetInputSourceProperty", "imageIndex": 0, "imageOffset": 10}]}],
        });
        std::fs::write(dir.join("speechify-test.ips"), format!("{{}}\n{raw}")).unwrap();
        let path = export(&dir, &dir, "test-version", Some(&dir)).unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        let bundle: serde_json::Value = serde_json::from_str(&contents).unwrap();
        assert_eq!(bundle["app_version"], "test-version");
        assert_eq!(bundle["logs"]["diagnostics.previous.log"]["status"], "missing");
        assert_eq!(bundle["logs"]["diagnostics.log"]["content"], "typing.paste begin\n");
        assert_eq!(
            bundle["native_crashes"][0]["frames"][0]["symbol"],
            "TSMGetInputSourceProperty"
        );
        assert!(!contents.contains("private"));
        std::fs::write(dir.join("diagnostics.previous.log"), vec![b'x'; LIMIT as usize + 100]).unwrap();
        let bounded = read_log(&dir.join("diagnostics.previous.log"));
        assert_eq!(bounded["status"], "truncated");
        assert_eq!(bounded["content"].as_str().unwrap().len(), LIMIT as usize);
        assert_eq!(read_log(&dir)["status"], "unreadable");
        assert!(export(&dir, &dir.join("nonexistent"), "test", None).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn panic_hook_does_not_relock_logger() {
        const CHILD_DIR: &str = "SPEECHIFY_PANIC_TEST_CHILD";
        if let Some(dir) = std::env::var_os(CHILD_DIR) {
            init(Path::new(&dir)).unwrap();
            let _guard = LOG.get().unwrap().lock().unwrap();
            panic!("private panic payload");
        }
        let dir = std::env::temp_dir().join(format!("speechify-panic-{}", std::process::id()));
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("panic_hook_does_not_relock_logger")
            .env(CHILD_DIR, &dir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("panic hook blocked behind logger mutex");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        assert!(!status.success());
        assert!(!std::fs::read_to_string(dir.join("diagnostics.log"))
            .unwrap()
            .contains("private panic payload"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn writes_are_visible_before_close_and_rotation_keeps_one_previous_file() {
        let dir = std::env::temp_dir().join(format!("speechify-diagnostics-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("diagnostics.log");
        let mut log = Log::new(path.clone()).unwrap();
        log.write("capture begin\n").unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().ends_with("capture begin\n"));
        log.write(&"x".repeat(LIMIT as usize - 14)).unwrap();
        log.write("paste begin\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "paste begin\n");
        log.write(&"y".repeat(LIMIT as usize - 12)).unwrap();
        log.write("restart\n").unwrap();
        assert!(std::fs::read_to_string(path.with_extension("previous.log"))
            .unwrap()
            .starts_with("paste begin\n"));
        drop(log);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn last_begin_survives_exit_without_destructors() {
        const CHILD_DIR: &str = "SPEECHIFY_DIAGNOSTICS_TEST_CHILD";
        if let Some(dir) = std::env::var_os(CHILD_DIR) {
            init(Path::new(&dir)).unwrap();
            scope("typing.paste", || std::process::exit(23));
            unreachable!();
        }
        let dir = std::env::temp_dir().join(format!("speechify-diagnostics-exit-{}", std::process::id()));
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("last_begin_survives_exit_without_destructors")
            .env(CHILD_DIR, &dir)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(23));
        let contents = std::fs::read_to_string(dir.join("diagnostics.log")).unwrap();
        assert!(contents.contains("stage=typing.paste begin"));
        assert!(!contents.contains("stage=typing.paste end"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn instrumentation_preserves_values_and_errors() {
        let dir = std::env::temp_dir().join(format!("speechify-diagnostics-events-{}", std::process::id()));
        init(&dir).unwrap();
        assert_eq!(scope("test", || 7), 7);
        assert_eq!(attempt("test", || Ok::<_, &str>(42)), Ok(42));
        assert_eq!(attempt("test", || Err::<(), _>("private error")), Err("private error"));
        let contents = std::fs::read_to_string(dir.join("diagnostics.log")).unwrap();
        assert!(contents.contains("stage=test begin"));
        assert!(contents.contains("stage=test ok"));
        assert!(contents.contains("stage=test error"));
        assert!(contents.contains("stage=test end elapsed_ms="));
        assert!(!contents.contains("private error"));
        // The process-global logger owns the file until process exit.
        #[cfg(unix)]
        std::fs::remove_dir_all(dir).unwrap();
    }
}
