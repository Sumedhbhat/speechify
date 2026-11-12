//! A small blocking HTTP client for local model servers and downloads.

use crate::Result;
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::time::Duration;

pub struct Http {
    agent: ureq::Agent,
}

fn explain(url: &str, error: ureq::Error) -> String {
    let host = url.split("//").nth(1).unwrap_or(url).split('/').next().unwrap_or(url);
    match error {
        ureq::Error::Timeout(_) => format!("{host} took too long to answer"),
        ureq::Error::ConnectionFailed | ureq::Error::Io(_) | ureq::Error::HostNotFound => {
            format!("Could not reach {host}. Is it running?")
        }
        other => format!("Request to {host} failed: {other}"),
    }
}

type Response = ureq::http::Response<ureq::Body>;

impl Http {
    /// `timeout` covers the whole request; downloads instead bound each read.
    pub fn new(timeout: Option<Duration>) -> Http {
        Self::with_timeouts(timeout, timeout.is_none().then_some(Duration::from_secs(30)))
    }

    fn with_timeouts(timeout: Option<Duration>, per_read: Option<Duration>) -> Http {
        let config = ureq::Agent::config_builder()
            .timeout_global(timeout)
            .timeout_per_read(per_read)
            .http_status_as_error(false)
            .build();
        Http { agent: config.into() }
    }

    fn check(url: &str, mut response: Response) -> Result<Response> {
        let status = response.status().as_u16();
        if (200..300).contains(&status) {
            return Ok(response);
        }
        let body = response.body_mut().read_to_string().unwrap_or_default();
        // Model servers put the reason in {"error": "..."} or {"error": {"message": "..."}}.
        let detail = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|json| {
                let error = &json["error"];
                error.as_str().or_else(|| error["message"].as_str()).map(str::to_string)
            })
            .unwrap_or_else(|| body.chars().take(200).collect());
        Err(format!("{url} answered {status}: {}", detail.trim()))
    }

    fn json(url: &str, response: Response) -> Result<Value> {
        let text = Self::check(url, response)?
            .body_mut()
            .with_config()
            .limit(64 << 20)
            .read_to_string()
            .map_err(|e| explain(url, e))?;
        serde_json::from_str(&text).map_err(|_| format!("{url} did not answer with JSON"))
    }

    pub fn get_json(&self, url: &str, key: &str) -> Result<Value> {
        let mut request = self.agent.get(url);
        if !key.is_empty() {
            request = request.header("Authorization", format!("Bearer {key}"));
        }
        Self::json(url, request.call().map_err(|e| explain(url, e))?)
    }

    pub fn post_json(&self, url: &str, key: &str, body: &Value) -> Result<Value> {
        let mut request = self.agent.post(url);
        if !key.is_empty() {
            request = request.header("Authorization", format!("Bearer {key}"));
        }
        Self::json(url, request.send_json(body).map_err(|e| explain(url, e))?)
    }

    /// Posts a raw body, for multipart uploads.
    pub fn post_bytes(&self, url: &str, key: &str, content_type: &str, body: &[u8]) -> Result<Value> {
        let mut request = self.agent.post(url).header("Content-Type", content_type);
        if !key.is_empty() {
            request = request.header("Authorization", format!("Bearer {key}"));
        }
        Self::json(url, request.send(body).map_err(|e| explain(url, e))?)
    }

    /// Posts JSON and hands each line of a streamed JSON-lines answer to `on_line`.
    pub fn post_lines(&self, url: &str, body: &Value, on_line: &mut dyn FnMut(Value) -> Result<()>) -> Result<()> {
        let response = Self::check(url, self.agent.post(url).send_json(body).map_err(|e| explain(url, e))?)?;
        for line in BufReader::new(response.into_body().into_reader()).lines() {
            let line = line.map_err(|_| format!("The connection to {url} dropped"))?;
            if let Ok(json) = serde_json::from_str(&line) {
                on_line(json)?;
            }
        }
        Ok(())
    }

    /// Downloads to `dest`, resuming a partial `dest.part` file. `progress`
    /// receives (bytes so far, total or 0) and returns false to cancel.
    pub fn download(&self, url: &str, dest: &Path, progress: &mut dyn FnMut(u64, u64) -> bool) -> Result<()> {
        let part = dest.with_extension("part");
        let have = std::fs::metadata(&part).map(|meta| meta.len()).unwrap_or(0);
        let mut request = self.agent.get(url);
        if have > 0 {
            request = request.header("Range", format!("bytes={have}-"));
        }
        let response = Self::check(url, request.call().map_err(|e| explain(url, e))?)?;
        // A server that ignores Range sends the whole file again.
        let resumed = response.status().as_u16() == 206;
        let length: u64 = response
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let mut done = if resumed { have } else { 0 };
        let total = if length > 0 { done + length } else { 0 };
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(resumed)
            .truncate(!resumed)
            .open(&part)
            .map_err(|e| format!("Could not write {}: {e}", part.display()))?;
        let mut reader = response.into_body().into_reader();
        let mut buffer = vec![0u8; 1 << 16];
        loop {
            let read = reader
                .read(&mut buffer)
                .map_err(|_| "The download was interrupted. Try again to resume it.".to_string())?;
            if read == 0 {
                break;
            }
            file.write_all(&buffer[..read])
                .map_err(|e| format!("Could not write {}: {e}", part.display()))?;
            done += read as u64;
            if !progress(done, total) {
                return Err("Download cancelled".into());
            }
        }
        drop(file);
        std::fs::rename(&part, dest).map_err(|e| format!("Could not save {}: {e}", dest.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{temp_dir, Reply, Server};
    use serde_json::json;

    fn http() -> Http {
        Http::new(Some(Duration::from_secs(5)))
    }

    #[test]
    fn json_requests_carry_the_key_and_body() {
        let server = Server::start(vec![
            Reply::json(r#"{"ok":true}"#),
            Reply::json(r#"{"n":1}"#),
            Reply::json("{}"),
        ]);
        assert_eq!(
            http().post_json(&server.url("/a"), "secret", &json!({"x": 1})).unwrap(),
            json!({"ok": true})
        );
        assert_eq!(http().get_json(&server.url("/b"), "").unwrap(), json!({"n": 1}));
        http()
            .post_bytes(&server.url("/c"), "k", "text/plain", b"hello")
            .unwrap();
        http().get_json(&server.url("/d"), "k2").unwrap_err();
        let requests = server.requests();
        assert!(requests[0].head.starts_with("POST /a "));
        assert!(requests[0].head.to_lowercase().contains("authorization: bearer secret"));
        assert_eq!(
            serde_json::from_slice::<Value>(&requests[0].body).unwrap(),
            json!({"x": 1})
        );
        assert!(requests[1].head.starts_with("GET /b ") && !requests[1].head.to_lowercase().contains("authorization"));
        assert!(requests[2].head.to_lowercase().contains("content-type: text/plain"));
        assert_eq!(requests[2].body, b"hello");
    }

    #[test]
    fn errors_read_like_sentences() {
        let server = Server::start(vec![
            Reply::status(404, r#"{"error":"model 'x' not found"}"#),
            Reply::status(401, r#"{"error":{"message":"bad key"}}"#),
            Reply::status(500, "boom"),
            Reply::json("not json"),
        ]);
        let url = server.url("/v1");
        assert_eq!(
            http().get_json(&url, "").unwrap_err(),
            format!("{url} answered 404: model 'x' not found")
        );
        assert_eq!(
            http().get_json(&url, "").unwrap_err(),
            format!("{url} answered 401: bad key")
        );
        assert_eq!(
            http().post_json(&url, "", &json!({})).unwrap_err(),
            format!("{url} answered 500: boom")
        );
        assert_eq!(
            http().get_json(&url, "").unwrap_err(),
            format!("{url} did not answer with JSON")
        );
        let closed = server.url("/x");
        drop(server);
        assert!(http()
            .get_json(&closed, "")
            .unwrap_err()
            .starts_with("Could not reach 127.0.0.1:"));
        assert!(http()
            .get_json("http://name.invalid/x", "")
            .unwrap_err()
            .contains("name.invalid"));
        assert!(http()
            .get_json("nonsense", "")
            .unwrap_err()
            .starts_with("Request to nonsense failed"));
    }

    #[test]
    fn slow_servers_time_out() {
        let server = Server::start(vec![Reply::json("{}").delayed(600)]);
        let quick = Http::new(Some(Duration::from_millis(100)));
        assert!(quick
            .get_json(&server.url("/slow"), "")
            .unwrap_err()
            .ends_with("took too long to answer"));
    }

    #[test]
    fn streamed_lines() {
        let server = Server::start(vec![
            Reply::json("{\"status\":\"a\"}\nnoise\n{\"status\":\"b\"}\n"),
            Reply::json("{\"n\":1}\n{\"n\":2}\n"),
            Reply::status(500, "x"),
        ]);
        let mut seen = Vec::new();
        http()
            .post_lines(&server.url("/pull"), &json!({}), &mut |line| {
                seen.push(line["status"].as_str().unwrap().to_string());
                Ok(())
            })
            .unwrap();
        assert_eq!(seen, vec!["a", "b"]);
        assert_eq!(
            http()
                .post_lines(&server.url("/pull"), &json!({}), &mut |_| Err("stop".into()))
                .unwrap_err(),
            "stop"
        );
        assert!(http()
            .post_lines(&server.url("/pull"), &json!({}), &mut |_| Ok(()))
            .is_err());
    }

    #[test]
    fn downloads_resume_and_cancel() {
        let dir = temp_dir("download");
        let dest = dir.join("model.gguf");
        let server = Server::start(vec![
            Reply::json("0123456789"),
            Reply::status(206, "56789"),
            Reply::json("whole"),
            Reply::json("abc").claiming(10),
            Reply::status(404, "gone"),
            Reply::json("late"),
        ]);
        // Cancelled part-way: the partial file stays for next time.
        assert_eq!(
            http()
                .download(&server.url("/m"), &dest, &mut |_, _| false)
                .unwrap_err(),
            "Download cancelled"
        );
        assert!(!dest.exists());
        std::fs::write(dest.with_extension("part"), "01234").unwrap();
        let mut last = (0, 0);
        http()
            .download(&server.url("/m"), &dest, &mut |done, total| {
                last = (done, total);
                true
            })
            .unwrap();
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "0123456789");
        assert_eq!(last, (10, 10));
        assert!(server.requests()[1].head.to_lowercase().contains("range: bytes=5-"));
        // A server without range support restarts from zero.
        std::fs::write(dest.with_extension("part"), "stale").unwrap();
        http().download(&server.url("/m"), &dest, &mut |_, _| true).unwrap();
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "whole");
        assert!(http()
            .download(&server.url("/m"), &dest, &mut |_, _| true)
            .unwrap_err()
            .contains("interrupted"));
        assert!(http()
            .download(&server.url("/m"), &dest, &mut |_, _| true)
            .unwrap_err()
            .contains("404"));
        std::fs::remove_file(dest.with_extension("part")).ok();
        assert!(http()
            .download(&server.url("/m"), &dir.join("no/such/file"), &mut |_, _| { true })
            .unwrap_err()
            .starts_with("Could not write"));
    }

    #[test]
    fn stalled_download_body_times_out_and_keeps_the_partial_file() {
        use std::net::TcpListener;
        use std::sync::mpsc::channel;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/model", listener.local_addr().unwrap());
        let (release, wait) = channel::<()>();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            assert!(stream.read(&mut request).unwrap() > 0);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nabc")
                .unwrap();
            wait.recv().unwrap();
        });
        let http = Http::with_timeouts(None, Some(Duration::from_millis(100)));
        let dest = temp_dir("stalled-download").join("model.gguf");
        let error = http.download(&url, &dest, &mut |_, _| true).unwrap_err();
        release.send(()).unwrap();
        server.join().unwrap();
        assert!(error.contains("interrupted"));
        assert!(!dest.exists());
        assert_eq!(std::fs::read(dest.with_extension("part")).unwrap(), b"abc");
    }
}
