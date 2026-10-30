use crate::config::{owned, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
#[cfg(test)]
#[path = "../tests/unit/models.rs"]
mod behavior;
pub fn catalog() -> Vec<Value> {
    serde_json::from_str(include_str!("../resources/catalog.json")).unwrap()
}
pub fn environment(id: &str) -> Option<&'static str> {
    if id.starts_with("ft:") || id.starts_with("distil") {
        Some("whisper")
    } else if id.starts_with("kyutai-") {
        Some("transformers")
    } else {
        None
    }
}
pub fn spec(id: &str) -> Result<Value> {
    catalog()
        .into_iter()
        .find(|m| m["id"] == id)
        .ok_or(format!("Unknown catalog model: {id}"))
}
pub fn path(root: &Path, id: &str) -> Result<PathBuf> {
    if id.starts_with("ft:") {
        return owned(&root.join("models"), id.trim_start_matches("ft:"));
    }
    owned(&root.join("models"), &format!("{id}.gguf"))
}
pub fn verify(path: &Path, expected: &str, cancel: &AtomicBool) -> Result<()> {
    let mut f = std::fs::File::open(path).map_err(crate::config::err)?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 1024 * 128];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
        let n = f.read(&mut buf).map_err(crate::config::err)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    if h.finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
        != expected
    {
        return Err("SHA-256 mismatch; file was not installed".into());
    }
    Ok(())
}
pub fn install(
    root: &Path,
    id: &str,
    cancel: Arc<AtomicBool>,
    progress: impl Fn(Value),
) -> Result<()> {
    let s = spec(id)?;
    let g = &s["gguf"];
    if g.is_null() {
        return Err(
            "This model needs its optional Python environment; install it from Models first".into(),
        );
    }
    let dst = path(root, id)?;
    let url = format!(
        "https://huggingface.co/{}/resolve/{}/{}",
        g["repo"].as_str().unwrap(),
        g["revision"].as_str().unwrap(),
        g["file"].as_str().unwrap()
    );
    let total = g["size"].as_u64().unwrap();
    install_artifact(
        &dst,
        &url,
        total,
        g["sha256"].as_str().unwrap(),
        &cancel,
        |stage, bytes| {
            progress(json!({"id":id,"stage":stage,"bytes":bytes,"total":total}));
        },
    )
}
fn install_artifact(
    dst: &Path,
    url: &str,
    total: u64,
    sha256: &str,
    cancel: &AtomicBool,
    progress: impl Fn(&str, u64),
) -> Result<()> {
    let tmp = dst.with_extension("partial");
    let result = (|| {
        download(url, total, &tmp, cancel, |bytes| {
            progress("Downloading", bytes);
        })?;
        progress("Verifying SHA-256", total);
        if let Err(error) = verify(&tmp, sha256, cancel) {
            let _ = std::fs::remove_file(&tmp);
            return Err(error);
        }
        progress("Installing", total);
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
        std::fs::rename(&tmp, dst).map_err(crate::config::err)?;
        Ok(())
    })();
    // Network/disk failures retain the resumable prefix; cancellation removes it.
    if cancel.load(Ordering::Relaxed) {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}
fn download(
    url: &str,
    total: u64,
    tmp: &Path,
    cancel: &AtomicBool,
    progress: impl Fn(u64),
) -> Result<()> {
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(20))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(crate::config::err)?;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(tmp)
        .map_err(crate::config::err)?;
    let mut bytes = f.metadata().map_err(crate::config::err)?.len();
    if bytes > total {
        f.set_len(0).map_err(crate::config::err)?;
        bytes = 0;
    }
    f.seek(SeekFrom::Start(bytes)).map_err(crate::config::err)?;
    progress(bytes);
    while bytes < total {
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
        let before = bytes;
        let end = (bytes + 4 * 1024 * 1024 - 1).min(total - 1);
        let attempt = (|| {
            let mut response = client
                .get(url)
                .header(reqwest::header::RANGE, format!("bytes={bytes}-{end}"))
                .send()
                .and_then(|r| r.error_for_status())
                .map_err(crate::config::err)?;
            if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
                return Err(
                    "Model host did not honor a bounded range request; retry later".to_owned(),
                );
            }
            let expected = format!("bytes {bytes}-{end}/{total}");
            if response
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                != Some(expected.as_str())
            {
                return Err("Model host returned an unexpected content range".into());
            }
            let mut remaining = end - bytes + 1;
            let mut buf = [0u8; 65536];
            while remaining > 0 {
                if cancel.load(Ordering::Relaxed) {
                    return Err("Cancelled".into());
                }
                let want = remaining.min(buf.len() as u64) as usize;
                let n = response
                    .read(&mut buf[..want])
                    .map_err(crate::config::err)?;
                if n == 0 {
                    return Err("Download was truncated".into());
                }
                f.write_all(&buf[..n]).map_err(crate::config::err)?;
                bytes += n as u64;
                remaining -= n as u64;
            }
            Ok(())
        })();
        progress(bytes);
        if let Err(error) = attempt {
            // Resume progress even if a slow range exceeds its bounded request deadline.
            // A zero-progress failure returns to the UI, preserving the prefix for Retry.
            if bytes == before || cancel.load(Ordering::Relaxed) {
                return Err(error);
            }
        }
    }
    f.sync_all().map_err(crate::config::err)
}

pub fn list(root: &Path) -> Vec<Value> {
    let mut items = catalog();
    for m in &mut items {
        let id = m["id"].as_str().unwrap();
        m["installed"] = json!(
            path(root, id).is_ok_and(|p| p.is_file()) || root.join("models").join(id).is_dir()
        );
        m["runtime"] = json!(if m["gguf"].is_null() {
            "Optional Python exception"
        } else {
            "Native transcribe.cpp"
        });
    }
    if let Ok(dirs) = std::fs::read_dir(root.join("models")) {
        for d in dirs.flatten() {
            let p = d.path();
            if p.is_dir()
                && p.join("model.bin").exists()
                && d.file_name().to_string_lossy().starts_with("whisper-ft-")
            {
                let name = d.file_name().to_string_lossy().into_owned();
                items.push(json!({
                    "id": format!("ft:{name}"),
                    "name": format!("Your trained Whisper ({name})"),
                    "repo": p,
                    "languages": "as trained",
                    "runtime": "Optional Python exception",
                    "installed": true,
                    "finetune": true,
                }));
            }
        }
    }
    items
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checksum_and_cancel() {
        let p = std::env::temp_dir().join(format!("speechify-tauri-hash-{}", std::process::id()));
        std::fs::write(&p, b"abc").unwrap();
        let flag = AtomicBool::new(false);
        assert!(verify(
            &p,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            &flag
        )
        .is_ok());
        assert!(verify(&p, "bad", &flag).is_err());
        flag.store(true, Ordering::SeqCst);
        assert_eq!(verify(&p, "bad", &flag).unwrap_err(), "Cancelled");
        std::fs::remove_file(p).unwrap();
    }
    #[test]
    fn interrupted_range_resumes_without_discarding_existing_prefix() {
        use std::{
            io::{BufRead, BufReader},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/model", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for (start, body) in [(2, "cd"), (4, "efgh")] {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut request = String::new();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    request.push_str(&line);
                }
                assert!(request
                    .to_lowercase()
                    .contains(&format!("range: bytes={start}-7")));
                write!(stream, "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-7/8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", 8-start).unwrap();
            }
        });
        let tmp = std::env::temp_dir().join(format!("speechify-range-{}", std::process::id()));
        std::fs::write(&tmp, b"ab").unwrap();
        download(&url, 8, &tmp, &AtomicBool::new(false), |_| {}).unwrap();
        server.join().unwrap();
        assert_eq!(std::fs::read(&tmp).unwrap(), b"abcdefgh");
        std::fs::remove_file(tmp).unwrap();
    }
    #[test]
    fn catalog_is_complete() {
        let c = catalog();
        assert_eq!(c.len(), 14);
        assert_eq!(c.iter().filter(|m| !m["gguf"].is_null()).count(), 11);
    }
}
