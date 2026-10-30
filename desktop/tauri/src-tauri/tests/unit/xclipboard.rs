use super::*;
use crate::behavior_tests::{wait_until, Home};

#[test]
fn clipboard_helper_protocol_reports_refusals_corrupt_output_and_timeouts_then_allows_retry() {
    for (reply, reason) in [
        (
            "printf '{\"Err\":\"Clipboard changed\"}\\n'",
            "Clipboard changed",
        ),
        ("printf '\\377\\n'", "utf-8"),
        ("/bin/sleep 60", "timed out"),
    ] {
        let home = Home::new();
        let pid_file = home.0.join("clipboard.pid");
        let mut command = Command::new("/bin/sh");
        command.args([
            "-c",
            &format!("echo $$ > '{}'; read request; {reply}", pid_file.display()),
        ]);
        let error = begin_using(command, "Dictation 🦀", Duration::from_millis(100)).unwrap_err();
        assert!(error.to_lowercase().contains(&reason.to_lowercase()), "{error}");
        let pid = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse::<i32>()
            .unwrap();
        wait_until(|| unsafe { libc::kill(pid, 0) } == -1);
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "read request; printf '{\"Ok\":null}\\n'"]);
        begin_using(command, "Retry", Duration::from_secs(1)).unwrap();
    }
    assert!(begin_using(
        Command::new("/a/missing/clipboard-helper"),
        "text",
        Duration::from_secs(1)
    )
    .is_err());
}

#[test]
#[ignore = "Requires the isolated Xvfb display"]
fn a_disconnected_clipboard_requester_releases_ownership_and_allows_a_later_copy() {
    let _desktop = crate::behavior_tests::DESKTOP
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    crate::platform::write_clipboard("Previous copy").unwrap();
    let (tx, rx) = mpsc::sync_channel(1);
    drop(rx);
    serve(b"Cancelled requester".to_vec(), &tx).unwrap();
    crate::platform::write_clipboard("Newer copy").unwrap();
    assert_eq!(read("UTF8_STRING").unwrap(), b"Newer copy");
}
