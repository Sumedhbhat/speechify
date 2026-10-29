use crate::{config::Result, service::Service};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader},
    process::{Command, Stdio},
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};
pub fn install(s: &Arc<Service>, id: &str) -> Result<()> {
    let family = match id {
        "distil-whisper-large-v3" => "whisper",
        "kyutai-stt-1b" | "kyutai-stt-2.6b" => "transformers",
        "training" => "training",
        _ => return Err("Unknown optional model".into()),
    };
    // Synchronize briefly with inference, then let unrelated models keep running.
    // ensure() checks the marker under the worker lock before starting a Python worker.
    if family != "training" {
        let mut slot = s.worker.lock().unwrap();
        *s.installing_family.lock().unwrap() = Some(family.into());
        if slot
            .as_ref()
            .is_some_and(|w| w.environment.as_deref() == Some(family))
        {
            slot.take();
            s.state.lock().unwrap()["ready"] = json!(false);
        }
    }
    struct Installing<'a>(&'a Service);
    impl Drop for Installing<'_> {
        fn drop(&mut self) {
            self.0.installing_family.lock().unwrap().take();
        }
    }
    let _installing = Installing(s);
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
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn training_setup_does_not_wait_for_the_dictation_worker_lock() {
        let root =
            std::env::temp_dir().join(format!("speechify-installer-lock-{}", std::process::id()));
        for sub in ["audio", "models", "logs", "exports", "resources/native"] {
            std::fs::create_dir_all(root.join(sub)).unwrap();
        }
        std::fs::write(root.join("resources/install-exception.py"),
            "import pathlib,sys,time\nr=pathlib.Path(sys.argv[sys.argv.index('--root')+1])\n(r/'started').touch()\nfor _ in range(100):\n if (r/'finish').exists():break\n time.sleep(.05)\n").unwrap();
        let service = Service::new(root.clone(), root.join("resources/native")).unwrap();
        let slot = service.worker.lock().unwrap();
        let background = service.clone();
        let task = std::thread::spawn(move || install(&background, "training"));
        let start = Instant::now();
        while !root.join("started").exists() && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(10));
        }
        let started = root.join("started").exists();
        std::fs::write(root.join("finish"), "").unwrap();
        drop(slot);
        task.join().unwrap().unwrap();
        assert!(
            started,
            "training setup was blocked by the dictation worker mutex"
        );
        drop(service);
        std::fs::remove_dir_all(root).unwrap();
    }
}
