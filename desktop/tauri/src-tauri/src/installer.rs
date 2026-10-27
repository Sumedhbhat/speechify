use crate::{config::Result, service::Service};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader},
    process::{Command, Stdio},
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};
pub fn install(s: &Arc<Service>, id: &str) -> Result<()> {
    // Do not mutate a Python environment while a worker is using it. New clips
    // wait on this same model lock, behind the existing bounded capture queue.
    let mut slot = s.worker.lock().unwrap();
    slot.take();
    s.state.lock().unwrap()["ready"] = json!(false);
    let family = match id {
        "distil-whisper-large-v3" => "whisper",
        "kyutai-stt-1b" | "kyutai-stt-2.6b" => "transformers",
        "training" => "training",
        _ => return Err("Unknown optional model".into()),
    };
    s.state.lock().unwrap()["download"] = json!({"id":id,"stage":"Starting optional installer"});
    let mut cmd = Command::new(if cfg!(windows) { "python" } else { "python3" });
    cmd.arg(
        s.native
            .parent()
            .ok_or("Missing resources")?
            .join("install-exception.py"),
    )
    .args(["--family", family, "--root"])
    .arg(&s.root);
    if id != "training" {
        cmd.args(["--model", id]);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let (mut child, _owner) = crate::child_owner::spawn(&mut cmd)
        .map_err(|e|format!("Optional setup needs Python 3 and uv on PATH. Install those explicitly, then retry: {e}"))?;
    s.install_pid.store(child.id(), Ordering::SeqCst);
    let (tx, rx) = std::sync::mpsc::sync_channel(8);
    let stdout = child.stdout.take().unwrap();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else {
                break;
            };
            if let Ok(value) = serde_json::from_str::<Value>(&line) {
                if tx.send(value).is_err() {
                    break;
                }
            }
        }
    });
    let start = Instant::now();
    let result = loop {
        if s.cancel.load(Ordering::SeqCst)
            || s.stopping.load(Ordering::SeqCst)
            || start.elapsed() > Duration::from_secs(7200)
        {
            crate::worker::terminate(child.id());
            let _ = child.wait();
            break Err("Installation cancelled or timed out. Retry resumes completed environment setup and restarts incomplete model files.".into());
        }
        if let Ok(mut progress) = rx.recv_timeout(Duration::from_millis(100)) {
            progress["id"] = json!(id);
            s.state.lock().unwrap()["download"] = progress;
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                break if status.success() {
                    Ok(())
                } else {
                    Err("Optional installation failed. Check Python 3, uv, available disk space and network; Retry safely repairs partial setup.".into())
                }
            }
            Ok(None) => (),
            Err(e) => {
                crate::worker::terminate(child.id());
                let _ = child.wait();
                break Err(e.to_string());
            }
        }
    };
    s.install_pid.store(0, Ordering::SeqCst);
    result
}
