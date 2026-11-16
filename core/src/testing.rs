//! Shared test fixtures.

use crate::store::Dictation;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

/// A finished dictation of `text` into `app` at `created_at`, spoken at 120 words a minute.
pub fn dictation(text: &str, app: &str, created_at: i64) -> Dictation {
    let words = text.split_whitespace().count() as u32;
    Dictation {
        created_at,
        offset_min: 330,
        app: app.into(),
        raw: text.to_lowercase(),
        text: text.into(),
        words,
        duration_ms: words * 600,
        speech_ms: words * 500,
        asr_ms: 200,
        total_ms: 300,
        speech_model: "test".into(),
        ..Dictation::default()
    }
}

/// A fresh scratch directory under the build folder (the system temp
/// directory on the development machine has a small quota).
pub fn temp_dir(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let base = std::env::var_os("CARGO_TARGET_TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join(format!(
        "speechify-{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// One scripted HTTP response.
#[derive(Clone)]
pub struct Reply {
    status: u16,
    body: Vec<u8>,
    delay_ms: u64,
    claimed: Option<usize>,
}

impl Reply {
    pub fn json(body: &str) -> Reply {
        Reply::status(200, body)
    }

    pub fn status(status: u16, body: &str) -> Reply {
        Reply {
            status,
            body: body.as_bytes().to_vec(),
            delay_ms: 0,
            claimed: None,
        }
    }

    pub fn delayed(mut self, ms: u64) -> Reply {
        self.delay_ms = ms;
        self
    }

    /// Announces a longer body than is sent, like a dropped connection.
    pub fn claiming(mut self, length: usize) -> Reply {
        self.claimed = Some(length);
        self
    }
}

pub struct Request {
    /// Request line and headers.
    pub head: String,
    pub body: Vec<u8>,
}

/// A local HTTP server that answers requests with the scripted replies in
/// order and records what it was sent.
pub struct Server {
    port: u16,
    requests: std::sync::Arc<std::sync::Mutex<Vec<Request>>>,
}

impl Server {
    pub fn start(replies: Vec<Reply>) -> Server {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = requests.clone();
        std::thread::spawn(move || {
            for reply in replies {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut data = Vec::new();
                let mut buffer = [0u8; 4096];
                let (head, body) = loop {
                    let read = stream.read(&mut buffer).unwrap_or(0);
                    data.extend_from_slice(&buffer[..read]);
                    let text = String::from_utf8_lossy(&data).to_string();
                    if let Some(split) = text.find("\r\n\r\n") {
                        let head = text[..split].to_string();
                        let length = head
                            .lines()
                            .find_map(|line| {
                                line.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if data.len() >= split + 4 + length {
                            break (head, data[split + 4..split + 4 + length].to_vec());
                        }
                    }
                    if read == 0 {
                        break (text, vec![]);
                    }
                };
                log.lock().unwrap().push(Request { head, body });
                std::thread::sleep(std::time::Duration::from_millis(reply.delay_ms));
                let header = format!(
                    "HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    reply.status,
                    reply.claimed.unwrap_or(reply.body.len())
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&reply.body);
            }
        });
        Server { port, requests }
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    pub fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn requests(&self) -> std::sync::MutexGuard<'_, Vec<Request>> {
        // Give the server thread a moment to record the last request.
        std::thread::sleep(std::time::Duration::from_millis(20));
        self.requests.lock().unwrap()
    }
}

/// Deterministic white noise at `amplitude`, like a microphone's own hiss.
pub fn hiss(length: usize, amplitude: f32) -> Vec<f32> {
    let mut state = 0x9E37_79B9u32;
    (0..length)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state as f32 / u32::MAX as f32 * 2.0 - 1.0) * amplitude
        })
        .collect()
}
