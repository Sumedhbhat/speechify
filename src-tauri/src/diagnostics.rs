//! Small, synchronous diagnostics: the last completed write survives a native abort.
//! Only fixed stage names and outcomes belong here, never user content or errors.
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
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
            .map(|at| format!("{}:{}", at.file(), at.line()))
            .unwrap_or_default();
        event("panic", &location);
        previous(info);
    }));
    Ok(())
}

pub fn event(stage: &str, outcome: &str) {
    let Some(log) = LOG.get() else { return };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let thread = std::thread::current().id();
    let line = format!(
        "{now} pid={} thread={thread:?} stage={stage} {outcome}\n",
        std::process::id()
    );
    if let Ok(mut log) = log.lock() {
        if log.write(&line).is_err() {
            eprintln!("Speechify could not write diagnostics");
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

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
