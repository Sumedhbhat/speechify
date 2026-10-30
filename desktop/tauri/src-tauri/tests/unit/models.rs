use super::*;
use crate::behavior_tests::Home;
use std::{
    io::{BufRead, BufReader},
    net::TcpListener,
    time::{Duration, Instant},
};
const ABC_SHA: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

fn host(response: &str) -> (String, std::thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/model.gguf", listener.local_addr().unwrap());
    let response = response.to_owned();
    let handle = std::thread::spawn(move || {
        let started = Instant::now();
        let (mut stream, _) = loop {
            if let Ok(connection) = listener.accept() {
                break connection;
            }
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "download did not contact its model host"
            );
            std::thread::sleep(Duration::from_millis(2));
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut request = String::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            request.push_str(&line);
        }
        stream.write_all(response.as_bytes()).unwrap();
        request
    });
    (url, handle)
}
const GOOD: &str="HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-2/3\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc";

#[test]
fn verified_download_is_committed_atomically_and_reports_its_stages() {
    let home = Home::new();
    let dest = home.0.join("models/model.gguf");
    let (url, server) = host(GOOD);
    let stages = std::sync::Mutex::new(vec![]);
    install_artifact(
        &dest,
        &url,
        3,
        ABC_SHA,
        &AtomicBool::new(false),
        |stage, bytes| {
            assert!(
                !dest.exists(),
                "model became visible before checksum verification completed"
            );
            stages.lock().unwrap().push((stage.to_owned(), bytes));
        },
    )
    .unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), b"abc");
    assert!(!dest.with_extension("partial").exists());
    assert_eq!(
        stages.into_inner().unwrap(),
        vec![
            ("Downloading".into(), 0),
            ("Downloading".into(), 3),
            ("Verifying SHA-256".into(), 3),
            ("Installing".into(), 3)
        ]
    );
    assert!(server
        .join()
        .unwrap()
        .to_lowercase()
        .contains("range: bytes=0-2"));
}

#[test]
fn checksum_failure_discards_the_partial_and_preserves_the_installed_model() {
    let home = Home::new();
    let dest = home.0.join("models/model.gguf");
    std::fs::write(&dest, b"previous model").unwrap();
    let (url, server) = host(GOOD);
    assert!(install_artifact(
        &dest,
        &url,
        3,
        "wrong hash",
        &AtomicBool::new(false),
        |_, _| {}
    )
    .unwrap_err()
    .contains("SHA-256 mismatch"));
    server.join().unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), b"previous model");
    assert!(!dest.with_extension("partial").exists());
}

#[test]
fn cancellation_removes_the_partial_without_contacting_the_model_host() {
    let home = Home::new();
    let dest = home.0.join("models/model.gguf");
    let flag = AtomicBool::new(true);
    assert_eq!(
        install_artifact(&dest, "http://127.0.0.1:1", 3, ABC_SHA, &flag, |_, _| {}).unwrap_err(),
        "Cancelled"
    );
    assert!(!dest.exists());
    assert!(!dest.with_extension("partial").exists());
    assert!(install(
        &home.0,
        "moonshine-base",
        Arc::new(AtomicBool::new(true)),
        |_| {}
    )
    .is_err());
}

#[test]
fn malformed_range_responses_are_refused_without_installing_or_losing_a_retry_prefix() {
    for (response,message) in [
        ("HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc","did not honor"),
        ("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 2-3/4\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc","unexpected content range"),
        ("HTTP/1.1 500 Failure\r\nContent-Length: 0\r\nConnection: close\r\n\r\n","500"),
        ("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-2/3\r\nContent-Length: 0\r\nConnection: close\r\n\r\n","truncated"),
    ] {
        let home=Home::new();let dest=home.0.join("models/model.gguf");let(url,server)=host(response);
        assert!(install_artifact(&dest,&url,3,ABC_SHA,&AtomicBool::new(false),|_,_|{}).unwrap_err().contains(message));
        server.join().unwrap();assert!(!dest.exists());assert!(dest.with_extension("partial").is_file());
    }
}

#[test]
fn stale_oversized_prefix_restarts_from_the_first_byte() {
    let home = Home::new();
    let dest = home.0.join("models/model.gguf");
    std::fs::write(dest.with_extension("partial"), b"oversized").unwrap();
    let (url, server) = host(GOOD);
    install_artifact(&dest, &url, 3, ABC_SHA, &AtomicBool::new(false), |_, _| {}).unwrap();
    assert!(server
        .join()
        .unwrap()
        .to_lowercase()
        .contains("range: bytes=0-2"));
    assert_eq!(std::fs::read(dest).unwrap(), b"abc");
}

#[test]
fn cancellation_after_verification_never_commits_a_model_and_retry_can_install_it() {
    let home = Home::new();
    let dest = home.0.join("models/test.gguf");
    std::fs::write(&dest, b"previous installation").unwrap();
    let (url, server) = host(GOOD);
    let cancel = AtomicBool::new(false);
    let error = install_artifact(&dest, &url, 3, ABC_SHA, &cancel, |stage, _| {
        if stage == "Installing" {
            cancel.store(true, Ordering::SeqCst);
        }
    })
    .unwrap_err();
    server.join().unwrap();
    assert_eq!(error, "Cancelled");
    assert_eq!(std::fs::read(&dest).unwrap(), b"previous installation");
    assert!(!dest.with_extension("partial").exists());
    let (url, server) = host(GOOD);
    cancel.store(false, Ordering::SeqCst);
    install_artifact(&dest, &url, 3, ABC_SHA, &cancel, |_, _| {}).unwrap();
    server.join().unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), b"abc");
}

#[test]
fn a_range_response_ending_before_any_bytes_arrive_is_a_retryable_download_error() {
    let home = Home::new();
    let dest = home.0.join("models/test.gguf");
    let (url, server) = host(
        "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-2/3\r\nConnection: close\r\n\r\n",
    );
    let error =
        install_artifact(&dest, &url, 3, ABC_SHA, &AtomicBool::new(false), |_, _| {}).unwrap_err();
    server.join().unwrap();
    assert!(error.contains("truncated"), "{error}");
    assert!(!dest.exists());
    assert_eq!(std::fs::read(dest.with_extension("partial")).unwrap(), b"");
}
