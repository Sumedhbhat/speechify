use crate::config::{owned, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
pub fn catalog() -> Vec<Value> {
    serde_json::from_str(include_str!("../resources/catalog.json")).unwrap()
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
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 1024 * 128];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
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
        return Err("This model requires the optional isolated Python environment; see candidate-testing.md".into());
    }
    let dst = path(root, id)?;
    let tmp = dst.with_extension("partial");
    let result = (|| {
        let url = format!(
            "https://huggingface.co/{}/resolve/{}/{}",
            g["repo"].as_str().unwrap(),
            g["revision"].as_str().unwrap(),
            g["file"].as_str().unwrap()
        );
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(20))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| e.to_string())?;
        let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        let total = g["size"].as_u64().unwrap();
        let mut bytes = 0u64;
        // Bounded range requests bound cancellation even when a server stops sending.
        while bytes < total {
            if cancel.load(Ordering::Relaxed) {
                return Err("Cancelled".into());
            }
            let end = (bytes + 4 * 1024 * 1024 - 1).min(total - 1);
            let mut response = client
                .get(&url)
                .header(reqwest::header::RANGE, format!("bytes={bytes}-{end}"))
                .send()
                .and_then(|r| r.error_for_status())
                .map_err(|e| e.to_string())?;
            if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
                return Err("Model host did not honor a bounded range request; retry later".into());
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
                let n = response.read(&mut buf[..want]).map_err(|e| e.to_string())?;
                if n == 0 {
                    return Err("Download was truncated".into());
                }
                f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
                bytes += n as u64;
                remaining -= n as u64;
            }
            progress(json!({"id":id,"stage":"Downloading","bytes":bytes,"total":total}));
        }
        f.sync_all().map_err(|e| e.to_string())?;
        if bytes != total {
            return Err("Download was truncated".into());
        }
        progress(json!({"id":id,"stage":"Verifying SHA-256","bytes":bytes,"total":total}));
        verify(&tmp, g["sha256"].as_str().unwrap(), &cancel)?;
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
        std::fs::rename(&tmp, &dst).map_err(|e| e.to_string())?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
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
                items.push(json!({"id":format!("ft:{name}"),"name":format!("Your trained Whisper ({name})"),"repo":p,"languages":"as trained","runtime":"Optional Python exception","installed":true,"finetune":true}));
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
    fn catalog_is_complete() {
        let c = catalog();
        assert_eq!(c.len(), 14);
        assert_eq!(c.iter().filter(|m| !m["gguf"].is_null()).count(), 11);
    }
}
