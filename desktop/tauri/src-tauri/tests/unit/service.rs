use super::*;
use crate::behavior_tests::{fake_service, wait_until, Home};
use std::{
    io::{BufRead, BufReader, Read},
    net::TcpListener,
    sync::mpsc,
    time::Duration,
};

fn cleanup_peer(
    response: String,
    release: Option<mpsc::Receiver<()>>,
) -> (String, mpsc::Receiver<Value>, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/cleanup", listener.local_addr().unwrap());
    let (tx, request) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse::<usize>().unwrap();
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        tx.send(serde_json::from_slice(&body).unwrap()).unwrap();
        if let Some(release) = release {
            release.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        stream.write_all(response.as_bytes()).unwrap();
    });
    (url, request, server)
}
fn response(status: &str, body: &str) -> String {
    format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len())
}
fn configure_cleanup(service: &Service, url: &str) {
    let mut cfg = service.config.lock().unwrap().clone();
    cfg["llm_url"] = json!(url);
    cfg["llm_model"] = json!("cleaner");
    service.update(cfg).unwrap();
}

#[test]
fn optional_cleanup_receives_the_transcript_and_saves_the_returned_text() {
    let home = Home::new();
    let service = fake_service(&home, "");
    let (url, request, server) = cleanup_peer(
        response(
            "200 OK",
            r#"{"choices":[{"message":{"content":"GitHub is ready."}}]}"#,
        ),
        None,
    );
    configure_cleanup(&service, &url);
    let wav = home.0.join("input.wav");
    crate::audio::save(&wav, &[0.1; 160]).unwrap();
    let result = service.infer_path(&wav, true).unwrap();
    assert_eq!(result["text"], "GitHub is ready.");
    assert!(result["cleanup_warning"].is_null());
    let sent = request.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(sent["model"], "cleaner");
    assert_eq!(sent["temperature"], 0);
    assert_eq!(
        sent["messages"][1],
        json!({"role":"user","content":"get hub"})
    );
    let history = service.store.lock().unwrap().history("", "").unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0]["raw"], "get hub");
    assert_eq!(history[0]["pasted"], "GitHub is ready.");
    server.join().unwrap();
}

#[test]
fn failed_or_empty_remote_cleanup_never_discards_a_local_transcript() {
    for (status, body) in [
        ("500 Failure", "{}"),
        ("200 OK", "not json"),
        ("200 OK", "{}"),
        ("200 OK", r#"{"choices":[{"message":{"content":"   "}}]}"#),
    ] {
        let home = Home::new();
        let service = fake_service(&home, "");
        let (url, _request, server) = cleanup_peer(response(status, body), None);
        configure_cleanup(&service, &url);
        let wav = home.0.join("input.wav");
        crate::audio::save(&wav, &[0.1; 160]).unwrap();
        let result = service.infer_path(&wav, true).unwrap();
        assert_eq!(result["text"], "get hub");
        assert!(result["cleanup_warning"].is_string());
        assert_eq!(
            service.store.lock().unwrap().history("", "").unwrap().len(),
            1
        );
        server.join().unwrap();
    }
}

#[test]
fn cancellation_while_cleanup_is_running_prevents_late_history_or_transcript_updates() {
    let home = Home::new();
    let service = fake_service(&home, "");
    let (release_tx, release_rx) = mpsc::channel();
    let (url, request, server) = cleanup_peer(
        response(
            "200 OK",
            r#"{"choices":[{"message":{"content":"Must not be saved"}}]}"#,
        ),
        Some(release_rx),
    );
    configure_cleanup(&service, &url);
    let wav = home.0.join("input.wav");
    crate::audio::save(&wav, &[0.1; 160]).unwrap();
    let child = service.clone();
    let job = std::thread::spawn(move || child.infer_path(&wav, true));
    request.recv_timeout(Duration::from_secs(5)).unwrap();
    service.cancel_work();
    release_tx.send(()).unwrap();
    assert_eq!(job.join().unwrap().unwrap_err(), "Cancelled");
    server.join().unwrap();
    assert!(service
        .store
        .lock()
        .unwrap()
        .history("", "")
        .unwrap()
        .is_empty());
    assert_eq!(service.snapshot()["state"]["recent"], "");
}

#[test]
fn inference_failure_retries_once_on_cpu_and_records_only_the_successful_result() {
    let home = Home::new();
    let service = fake_service(&home, "gpu-error");
    let mut cfg = service.config.lock().unwrap().clone();
    cfg["device"] = json!("cuda");
    service.update(cfg).unwrap();
    let wav = home.0.join("input.wav");
    crate::audio::save(&wav, &[0.1; 160]).unwrap();
    let result = service.infer_path(&wav, true).unwrap();
    assert_eq!(result["text"], "get hub");
    assert_eq!(result["device"], "cpu");
    assert_eq!(
        service.store.lock().unwrap().history("", "").unwrap().len(),
        1
    );
}

#[test]
fn cancelling_an_inflight_worker_reports_cancellation_and_never_restarts_it() {
    let home = Home::new();
    let service = fake_service(&home, "slow");
    let wav = home.0.join("input.wav");
    crate::audio::save(&wav, &[0.1; 160]).unwrap();
    let child = service.clone();
    let job = std::thread::spawn(move || child.infer_path(&wav, true));
    wait_until(|| home.0.join("models/whisper-ft-test/request.json").exists());
    service.cancel_work();
    assert_eq!(job.join().unwrap().unwrap_err(), "Cancelled");
    assert!(service
        .store
        .lock()
        .unwrap()
        .history("", "")
        .unwrap()
        .is_empty());
    assert_eq!(service.snapshot()["state"]["stage"], "Cancelled");
}
