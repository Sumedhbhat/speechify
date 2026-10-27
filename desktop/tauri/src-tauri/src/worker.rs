use crate::{config::Result, native::Native};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};
pub fn serve(dir: &Path, model: &Path, device: &str, threads: i32) -> Result<()> {
    #[cfg(unix)]
    {
        let parent = unsafe { libc::getppid() };
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
            if unsafe { libc::getppid() } != parent {
                std::process::exit(1);
            }
        });
    }
    let t = Instant::now();
    let loaded = Native::open(dir, model, device, threads).or_else(|e| {
        if device != "cpu" {
            Native::open(dir, model, "cpu", threads)
                .map_err(|f| format!("GPU load failed ({e}); CPU load failed ({f})"))
        } else {
            Err(e)
        }
    })?;
    println!(
        "{}",
        json!({"ready":true,"device":loaded.device,"load_ms":t.elapsed().as_secs_f64()*1000.})
    );
    std::io::stdout().flush().map_err(|e| e.to_string())?;
    let mut engine = Some(loaded);
    for line in std::io::stdin().lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.len() > 32768 {
            return Err("Worker message too large".into());
        }
        let request: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
        let start = Instant::now();
        let result: Result<String> = (|| {
            let audio =
                crate::audio::load(Path::new(request["wav"].as_str().ok_or("Missing WAV")?))?;
            let lang = request["language"].as_str().unwrap_or("en");
            let prompt = request["prompt"].as_str().unwrap_or("");
            let first = engine
                .as_mut()
                .ok_or("Engine unavailable")?
                .run(&audio, lang, prompt);
            if first.is_err() && engine.as_ref().is_some_and(|e| e.device != "cpu") {
                engine.take();
                engine = Some(Native::open(dir, model, "cpu", threads)?);
                return engine.as_mut().unwrap().run(&audio, lang, prompt);
            }
            first
        })();
        let response = match result {
            Ok(text) => {
                json!({"text":text,"device":engine.as_ref().map(|e|&e.device),"inference_ms":start.elapsed().as_secs_f64()*1000.})
            }
            Err(error) => json!({"error":error}),
        };
        println!("{response}");
        std::io::stdout().flush().map_err(|e| e.to_string())?;
    }
    Ok(())
}
pub fn terminate(pid: u32) {
    if pid == 0 {
        return;
    }
    #[cfg(unix)]
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}
pub struct Worker {
    child: Child,
    _owner: crate::child_owner::Owner,
    input: std::process::ChildStdin,
    output: std::sync::mpsc::Receiver<Result<String>>,
    pub key: String,
    pub device: String,
    pub load_ms: f64,
    pid: Arc<AtomicU32>,
}
impl Worker {
    pub fn start(
        dir: &Path,
        model: &Path,
        device: &str,
        threads: i32,
        key: String,
        pid: Arc<AtomicU32>,
        python: Option<&Path>,
        config: &Value,
    ) -> Result<Self> {
        let mut cmd = if let Some(py) = python {
            let mut c = Command::new(py);
            c.arg(dir.parent().unwrap_or(dir).join("exception-worker.py"))
                .arg(model)
                .arg(device)
                .arg(config.to_string());
            c
        } else {
            let mut c = Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
            c.args(["--native-worker"])
                .arg(dir)
                .arg(model)
                .arg(device)
                .arg(threads.to_string());
            c
        };
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let (mut child, owner) = crate::child_owner::spawn(&mut cmd)?;
        pid.store(child.id(), Ordering::SeqCst);
        let input = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (send, output) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                let result = (&mut reader).take(1024 * 1024).read_line(&mut line);
                match result {
                    Ok(0) => break,
                    Ok(_) => {
                        if send.send(Ok(line)).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = send.send(Err(e.to_string()));
                        break;
                    }
                }
            }
        });
        let mut worker = Self {
            child,
            _owner: owner,
            input,
            output,
            key,
            device: String::new(),
            load_ms: 0.,
            pid,
        };
        let msg = worker.read()?;
        if msg["ready"] != true {
            return Err(msg.to_string());
        }
        worker.device = msg["device"].as_str().unwrap_or("unknown").into();
        worker.load_ms = msg["load_ms"].as_f64().unwrap_or(0.);
        Ok(worker)
    }
    pub fn alive(&mut self) -> bool {
        self.child.try_wait().is_ok_and(|status| status.is_none())
    }
    fn read(&mut self) -> Result<Value> {
        let line = self
            .output
            .recv_timeout(std::time::Duration::from_secs(180))
            .map_err(|e| {
                terminate(self.child.id());
                format!("Worker stopped responding: {e}; retry to recover")
            })??;
        let v: Value =
            serde_json::from_str(&line).map_err(|e| format!("Invalid worker response: {e}"))?;
        if let Some(e) = v["error"].as_str() {
            return Err(e.into());
        }
        Ok(v)
    }
    pub fn run(&mut self, wav: &Path, language: &str, prompt: &str) -> Result<Value> {
        writeln!(
            self.input,
            "{}",
            json!({"wav":wav,"language":language,"prompt":prompt})
        )
        .and_then(|_| self.input.flush())
        .map_err(|e| e.to_string())?;
        self.read()
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        terminate(self.child.id());
        let _ = self.child.wait();
        self.pid
            .compare_exchange(self.child.id(), 0, Ordering::SeqCst, Ordering::SeqCst)
            .ok();
    }
}
pub type WorkerSlot = Arc<Mutex<Option<Worker>>>;
pub fn native_dir(resource: Option<PathBuf>) -> PathBuf {
    if let Some(p) = std::env::var_os("SPEECHIFY_NATIVE_DIR") {
        return p.into();
    }
    if let Some(p) = resource {
        let native = p.join("resources/native");
        if native.is_dir() {
            return native;
        }
    }
    let exe = std::env::current_exe().unwrap();
    let parent = exe.parent().unwrap();
    let mut candidates = vec![parent.join("resources/native")];
    #[cfg(target_os = "linux")]
    if let Some(prefix) = parent.parent() {
        candidates.push(prefix.join("lib/Speechify Tauri Candidate/resources/native"));
    }
    #[cfg(target_os = "macos")]
    if let Some(contents) = parent.parent() {
        candidates.push(contents.join("Resources/resources/native"));
    }
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/native"));
    candidates
        .into_iter()
        .find(|p| p.is_dir())
        .unwrap_or_else(|| parent.join("resources/native"))
}
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn cancellation_reaps_process_group() {
        use std::os::unix::process::CommandExt;
        let mut child = Command::new("sh")
            .args(["-c", "sleep 60 & wait"])
            .process_group(0)
            .spawn()
            .unwrap();
        let start = Instant::now();
        terminate(child.id());
        assert!(!child.wait().unwrap().success());
        assert!(start.elapsed().as_secs() < 2);
    }
}
