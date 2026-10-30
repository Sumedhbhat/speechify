use super::*;
use crate::behavior_tests::{fake_service, wait_until, Home};

fn script(home: &Home, body: &str) {
    std::fs::write(home.0.join("resources/install-exception.py"), body).unwrap();
}
const WAIT:&str="import pathlib,sys,time\nr=pathlib.Path(sys.argv[sys.argv.index('--root')+1])\nprint('installer log before progress',flush=True)\nprint('{\"stage\":\"Installing test environment\"}',flush=True)\nwhile not (r/'finish').exists():time.sleep(.01)\n(r/'installed').write_text('ready')\n";

#[test]
fn installer_log_noise_never_panics_or_hides_subsequent_valid_progress() {
    for noise in ["json-scalars", "non-utf8"] {
        let home = Home::new();
        let service = home.service();
        std::fs::write(home.0.join("noise"), noise).unwrap();
        script(
            &home,
            r#"import pathlib,sys,time
r=pathlib.Path(sys.argv[sys.argv.index('--root')+1])
print('{"stage":"Starting fixture"}',flush=True)
if (r/'noise').read_text() == 'non-utf8':
 sys.stdout.buffer.write(b'\xff\n');sys.stdout.buffer.flush()
else:
 for value in ['42','null','[]','true','"a log"','{}','{"stage":9}']:print(value,flush=True)
print('{"stage":"Fixture ready","percent":50,"id":"another job"}',flush=True)
while not (r/'finish').exists():time.sleep(.01)
"#,
        );
        let background = service.clone();
        let job = std::thread::spawn(move || install(&background, "training"));
        let start = Instant::now();
        while !job.is_finished()
            && service.snapshot()["state"]["download"]["stage"] != "Fixture ready"
            && start.elapsed() < Duration::from_secs(3)
        {
            std::thread::sleep(Duration::from_millis(5));
        }
        let progress = service.snapshot()["state"]["download"].clone();
        std::fs::write(home.0.join("finish"), "").unwrap();
        let result = job.join();
        assert!(result.is_ok(), "installer panicked on {noise}");
        result.unwrap().unwrap();
        assert_eq!(
            progress["stage"], "Fixture ready",
            "lost progress after {noise}"
        );
        assert_eq!(progress["percent"], 50);
        assert_eq!(progress["id"], "training");
        assert_eq!(service.install_pid.load(Ordering::SeqCst), 0);
        assert!(service.installing_family.lock().unwrap().is_none());
    }
}

#[test]
fn optional_install_reports_progress_then_releases_its_family_and_process() {
    let home = Home::new();
    let service = home.service();
    script(&home, WAIT);
    assert!(install(&service, "not-a-model").is_err());
    let child = service.clone();
    let job = std::thread::spawn(move || install(&child, "kyutai-stt-1b"));
    wait_until(|| {
        service.snapshot()["state"]["download"]["stage"] == "Installing test environment"
    });
    assert_eq!(
        service.snapshot()["state"]["download"]["id"],
        "kyutai-stt-1b"
    );
    assert_eq!(
        service.installing_family.lock().unwrap().as_deref(),
        Some("transformers")
    );
    assert_ne!(service.install_pid.load(Ordering::SeqCst), 0);
    std::fs::write(home.0.join("finish"), "").unwrap();
    job.join().unwrap().unwrap();
    assert_eq!(
        std::fs::read_to_string(home.0.join("installed")).unwrap(),
        "ready"
    );
    assert!(service.installing_family.lock().unwrap().is_none());
    assert_eq!(service.install_pid.load(Ordering::SeqCst), 0);
}

#[test]
fn failed_optional_install_can_be_retried_without_restarting_the_service() {
    let home = Home::new();
    let service = home.service();
    script(&home, "raise SystemExit(3)\n");
    assert!(install(&service, "distil-whisper-large-v3")
        .unwrap_err()
        .contains("Retry"));
    assert!(service.installing_family.lock().unwrap().is_none());
    assert_eq!(service.install_pid.load(Ordering::SeqCst), 0);
    script(&home,"import pathlib,sys\nr=pathlib.Path(sys.argv[sys.argv.index('--root')+1])\n(r/'repaired').touch()\n");
    install(&service, "distil-whisper-large-v3").unwrap();
    assert!(home.0.join("repaired").exists());
}

#[test]
fn cancellation_stops_the_owned_installer_and_leaves_no_installed_result() {
    let home = Home::new();
    let service = home.service();
    script(&home, WAIT);
    let child = service.clone();
    let job = std::thread::spawn(move || install(&child, "training"));
    wait_until(|| {
        service.snapshot()["state"]["download"]["stage"] == "Installing test environment"
    });
    let pid = service.install_pid.load(Ordering::SeqCst);
    service.cancel_work();
    assert!(job.join().unwrap().unwrap_err().contains("cancelled"));
    assert_eq!(service.install_pid.load(Ordering::SeqCst), 0);
    assert!(!home.0.join("installed").exists());
    #[cfg(unix)]
    assert_eq!(
        unsafe { libc::kill(pid as i32, 0) },
        -1,
        "installer process survived cancellation"
    );
    #[cfg(windows)]
    unsafe {
        use std::ffi::c_void;
        #[link(name = "kernel32")]
        extern "system" {
            fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
            fn GetExitCodeProcess(process: *mut c_void, code: *mut u32) -> i32;
            fn CloseHandle(handle: *mut c_void) -> i32;
        }
        let process = OpenProcess(0x1000, 0, pid);
        if !process.is_null() {
            let mut code = 259;
            GetExitCodeProcess(process, &mut code);
            CloseHandle(process);
            assert_ne!(code, 259, "installer process survived cancellation");
        }
    }
}

#[test]
fn reinstalling_an_optional_family_stops_only_workers_using_that_family() {
    for (id, stops) in [("distil-whisper-large-v3", true), ("kyutai-stt-1b", false)] {
        let home = Home::new();
        let service = fake_service(&home, "");
        script(&home, WAIT);
        service.preload().unwrap();
        let before = service.pid.load(Ordering::SeqCst);
        let child = service.clone();
        let job = std::thread::spawn(move || install(&child, id));
        wait_until(|| {
            service.snapshot()["state"]["download"]["stage"] == "Installing test environment"
        });
        if stops {
            assert_eq!(service.pid.load(Ordering::SeqCst), 0);
            assert!(service.preload().unwrap_err().contains("being installed"));
        } else {
            service.preload().unwrap();
            assert_eq!(service.pid.load(Ordering::SeqCst), before);
        }
        std::fs::write(home.0.join("finish"), "").unwrap();
        job.join().unwrap().unwrap();
        service.preload().unwrap();
        assert!(service.snapshot()["state"]["ready"].as_bool().unwrap());
    }
}

#[cfg(unix)]
#[test]
fn missing_runtime_and_lost_child_ownership_are_reported_without_latching_installation() {
    let home = Home::new();
    let mode = match std::env::var("SPEECHIFY_TEST_INSTALLER_BOUNDARY") {
        Ok(mode) => mode,
        Err(_) => {
            for mode in ["missing-runtime", "reaped"] {
                let mut child = std::process::Command::new(std::env::current_exe().unwrap());
                child.args(["--exact", "installer::behavior::missing_runtime_and_lost_child_ownership_are_reported_without_latching_installation"])
                    .env("SPEECHIFY_TEST_INSTALLER_BOUNDARY", mode);
                if mode == "missing-runtime" {
                    child.env("PATH", &home.0);
                }
                let output = child.output().unwrap();
                assert!(
                    output.status.success(),
                    "{mode}: {}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            return;
        }
    };
    if mode == "reaped" {
        unsafe {
            libc::signal(libc::SIGCHLD, libc::SIG_IGN);
        }
    }
    let service = home.service();
    script(&home, "pass\n");
    let error = install(&service, "distil-whisper-large-v3").unwrap_err();
    if mode == "missing-runtime" {
        assert!(error.contains("Python 3 and uv"), "{error}");
    } else {
        assert_eq!(
            error,
            std::io::Error::from_raw_os_error(libc::ECHILD).to_string()
        );
    }
    assert_eq!(service.install_pid.load(Ordering::SeqCst), 0);
    assert!(service.installing_family.lock().unwrap().is_none());
}

#[test]
fn cancellation_before_setup_stops_the_installer_and_allows_a_later_retry() {
    let home = Home::new();
    let service = home.service();
    script(&home, WAIT);
    service.cancel_work();
    assert!(install(&service, "training")
        .unwrap_err()
        .contains("cancelled"));
    assert_eq!(service.install_pid.load(Ordering::SeqCst), 0);
    assert!(!home.0.join("installed").exists());
    service.cancel.store(false, Ordering::SeqCst);
    script(&home, "pass\n");
    install(&service, "training").unwrap();
}

#[test]
fn a_noisy_installer_can_finish_without_leaving_a_busy_process_or_a_blocked_progress_reader() {
    let home = Home::new();
    let service = home.service();
    script(&home, "import pathlib,sys\nfor _ in range(2000):print('{\"stage\":\"Installing\"}')\npathlib.Path(sys.argv[sys.argv.index('--root')+1]).joinpath('installed').touch()\n");
    install(&service, "training").unwrap();
    assert!(home.0.join("installed").exists());
    assert_eq!(service.install_pid.load(Ordering::SeqCst), 0);
    assert!(service.installing_family.lock().unwrap().is_none());
}
