//! The speech models the app can download, pinned by revision and checksum.

use crate::http::Http;
use crate::settings::DEFAULTS;
use crate::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub file: String,
    pub url: String,
    pub sha256: String,
    pub bytes: u64,
    /// One line for the model picker.
    pub notes: String,
    /// Too slow to enjoy without a graphics card.
    pub wants_gpu: bool,
    /// Made to recognise speech as it is spoken, for the streaming setup.
    pub streaming: bool,
}

/// One catalog row: a model in the handy-computer GGUF collection.
struct Pin {
    id: &'static str,
    name: &'static str,
    /// Git revision of the Hugging Face repository `<id>-gguf`.
    revision: &'static str,
    sha256: &'static str,
    bytes: u64,
    wants_gpu: bool,
    streaming: bool,
    notes: &'static str,
}

const PINS: &[Pin] = &[
    Pin {
        id: "parakeet-unified-en-0.6b",
        name: "Parakeet Unified 0.6B (English)",
        revision: "843b1b175ace04f5a89d379c79e2518c0d065b25",
        sha256: "4b50b6dd862bf6e346929aaf4f5eaacec003bfa3f56462d6c874b41ef2f38795",
        bytes: 731_357_568,
        wants_gpu: true,
        streaming: true,
        notes: "Recognises you as you speak, so the text is ready almost as soon as you stop. English only.",
    },
    Pin {
        id: "whisper-large-v3",
        name: "Whisper large-v3",
        revision: "274ad1d1cf680b7ee53cc70f78461394cd3a344b",
        sha256: "2fa1a5f179f8a511a53e2108db270aa4af3ce08cd976af4180e2854666bb4ba3",
        bytes: 1_668_741_440,
        wants_gpu: true,
        streaming: false,
        notes: "The most accurate model. Reviews the day's dictations overnight, when nobody is waiting.",
    },
];

/// Models converted for transcribe.cpp, 8-bit quantised.
pub fn models() -> Vec<Model> {
    PINS.iter()
        .map(|pin| {
            let file = format!("{}-Q8_0.gguf", pin.id);
            Model {
                id: pin.id.into(),
                name: pin.name.into(),
                url: format!(
                    "https://huggingface.co/handy-computer/{}-gguf/resolve/{}/{file}",
                    pin.id, pin.revision
                ),
                file,
                sha256: pin.sha256.into(),
                bytes: pin.bytes,
                notes: pin.notes.into(),
                wants_gpu: pin.wants_gpu,
                streaming: pin.streaming,
            }
        })
        .collect()
}

pub fn find(id: &str) -> Result<Model> {
    models()
        .into_iter()
        .find(|model| model.id == id)
        .ok_or_else(|| format!("Unknown speech model \"{id}\""))
}

/// The model dictation uses.
pub fn dictation() -> Model {
    find(&DEFAULTS.speech_model).expect("core/defaults.json names a catalog model")
}

/// The model the nightly review uses.
pub fn teacher() -> Model {
    find(&DEFAULTS.teacher_speech_model).expect("core/defaults.json names a catalog model")
}

/// What kind of compute device a model could run on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceClass {
    Cpu,
    /// A graphics card with its own memory.
    Gpu,
    /// Graphics built into the processor, sharing system memory.
    IntegratedGpu,
    /// Anything else (BLAS accelerators and the like).
    Other,
}

impl DeviceClass {
    /// From the runtime's `transcribe_device_type` code.
    pub fn from_code(code: i32) -> DeviceClass {
        match code {
            0 => DeviceClass::Cpu,
            1 => DeviceClass::Gpu,
            2 => DeviceClass::IntegratedGpu,
            _ => DeviceClass::Other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub name: String,
    /// The backend: "cpu", "vulkan", "cuda", "metal" and so on.
    pub kind: String,
    pub memory_mb: u64,
    pub class: DeviceClass,
}

/// What first start chose for this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Plan {
    /// Language-model cleanup: only worth its delay on a graphics card.
    pub rewrite: bool,
    /// One sentence for the user on why these were picked.
    pub reason: String,
}

/// Enough graphics memory for the speech model plus a 4B language model.
const GPU_MEMORY_MB: u64 = 4000;

/// Dictation always uses the one streaming model; what depends on the
/// machine is whether language-model cleanup can keep up. With a graphics
/// card it adds well under a second; without one, the rules alone are used.
pub fn plan(devices: &[Device]) -> Plan {
    let card = devices
        .iter()
        .filter(|device| device.class == DeviceClass::Gpu && device.memory_mb >= GPU_MEMORY_MB)
        .max_by_key(|device| device.memory_mb);
    match card {
        Some(card) => Plan {
            rewrite: true,
            reason: format!(
                "Found {} with {} GB: running {} and the cleanup model on it",
                card.name,
                card.memory_mb / 1000,
                dictation().name
            ),
        },
        None => Plan {
            rewrite: false,
            reason: format!(
                "No graphics card with enough memory: running {} without the cleanup model",
                dictation().name
            ),
        },
    }
}

/// On the very first start, before any settings exist, chooses what this
/// machine can run and saves it. Later starts keep the user's choices.
pub fn first_run(store: &crate::store::Store, devices: &[Device]) -> Result<Option<Plan>> {
    if store.get("settings").is_some() {
        return Ok(None);
    }
    let chosen = plan(devices);
    let mut settings = crate::settings::Settings::default();
    settings.rewrite.enabled = chosen.rewrite;
    store.save_settings(&settings)?;
    Ok(Some(chosen))
}

pub fn path(dir: &Path, model: &Model) -> PathBuf {
    dir.join(&model.file)
}

/// Whether the complete file is on disk. Size is checked here; the checksum
/// was verified when the download finished.
pub fn installed(dir: &Path, model: &Model) -> bool {
    std::fs::metadata(path(dir, model)).is_ok_and(|meta| meta.len() == model.bytes)
}

fn sha256(file: &Path) -> Result<String> {
    let mut reader = std::fs::File::open(file).map_err(|e| format!("Could not read {}: {e}", file.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|e| format!("Could not read {}: {e}", file.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Downloads a model into `dir`, resuming a partial download, and verifies it.
/// `progress` receives (bytes so far, total) and returns false to cancel.
pub fn download(dir: &Path, model: &Model, progress: &mut dyn FnMut(u64, u64) -> bool) -> Result<PathBuf> {
    std::fs::create_dir_all(dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    let dest = path(dir, model);
    if installed(dir, model) {
        return Ok(dest);
    }
    Http::new(None).download(&model.url, &dest, &mut |done, total| {
        progress(done, if total > 0 { total } else { model.bytes })
    })?;
    if sha256(&dest)? != model.sha256 {
        let _ = std::fs::remove_file(&dest);
        return Err(format!("{} arrived damaged. Download it again.", model.name));
    }
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{temp_dir, Reply, Server};

    #[test]
    fn catalog_is_consistent() {
        let all = models();
        assert_eq!(all.len(), 2);
        for model in &all {
            assert_eq!(model.sha256.len(), 64);
            assert!(
                model.url.starts_with("https://huggingface.co/handy-computer/") && model.url.ends_with(&model.file)
            );
            assert!(
                model.url.contains("/resolve/") && !model.url.contains("/main/"),
                "{} must be pinned",
                model.id
            );
            assert!(model.bytes > 100_000_000 && !model.notes.is_empty());
        }
        assert_eq!(find("whisper-large-v3").unwrap().name, "Whisper large-v3");
        assert_eq!(find("nope").unwrap_err(), "Unknown speech model \"nope\"");

        // Dictation streams; the teacher wants the most accurate model.
        assert!(dictation().streaming);
        assert_eq!(dictation().id, "parakeet-unified-en-0.6b");
        assert!(!teacher().streaming);
        assert_eq!(teacher().id, "whisper-large-v3");
    }

    fn device(class: DeviceClass, memory_mb: u64, name: &str) -> Device {
        Device {
            name: name.into(),
            kind: "vulkan".into(),
            memory_mb,
            class,
        }
    }

    #[test]
    fn only_a_graphics_card_gets_the_cleanup_model() {
        let cpu = device(DeviceClass::Cpu, 16_000, "AMD Ryzen 5 3500U");
        let igpu = device(DeviceClass::IntegratedGpu, 8000, "AMD Radeon Vega 8");
        let small = device(DeviceClass::Gpu, 2048, "GeForce MX150");
        let rtx = device(DeviceClass::Gpu, 8188, "NVIDIA GeForce RTX 4060 Laptop GPU");
        let bigger = device(DeviceClass::Gpu, 12_288, "NVIDIA GeForce RTX 4070");
        let laptop = plan(&[cpu.clone(), rtx.clone()]);
        assert!(laptop.rewrite);
        assert_eq!(
            laptop.reason,
            format!(
                "Found NVIDIA GeForce RTX 4060 Laptop GPU with 8 GB: running {} and the cleanup model on it",
                dictation().name
            )
        );
        assert!(plan(&[rtx, bigger]).reason.contains("RTX 4070"));
        // Integrated graphics, a small card or no card at all: no cleanup model.
        for machine in [vec![cpu.clone()], vec![cpu.clone(), igpu], vec![cpu, small], vec![]] {
            let chosen = plan(&machine);
            assert!(!chosen.rewrite);
            assert!(chosen.reason.starts_with("No graphics card"));
        }
        // Devices travel as JSON from the probe process.
        let json = serde_json::to_string(&[device(DeviceClass::Gpu, 8188, "RTX")]).unwrap();
        assert_eq!(
            serde_json::from_str::<Vec<Device>>(&json).unwrap()[0].class,
            DeviceClass::Gpu
        );
        let codes: Vec<DeviceClass> = (0..5).map(DeviceClass::from_code).collect();
        assert_eq!(
            codes,
            [
                DeviceClass::Cpu,
                DeviceClass::Gpu,
                DeviceClass::IntegratedGpu,
                DeviceClass::Other,
                DeviceClass::Other
            ]
        );
    }

    #[test]
    fn the_first_start_chooses_cleanup_and_later_starts_keep_it() {
        let store = crate::store::Store::memory();
        let chosen = first_run(&store, &[]).unwrap().unwrap();
        assert!(!chosen.rewrite && !store.settings().rewrite.enabled);
        // The user changes their mind; a later start must not undo that.
        let mut changed = store.settings();
        changed.rewrite.enabled = true;
        store.save_settings(&changed).unwrap();
        assert_eq!(
            first_run(&store, &[device(DeviceClass::Gpu, 8000, "RTX")]).unwrap(),
            None
        );
        assert!(store.settings().rewrite.enabled);
    }

    fn tiny(server: &Server, sha256: &str) -> Model {
        Model {
            id: "tiny".into(),
            name: "Tiny".into(),
            file: "tiny.gguf".into(),
            url: server.url("/tiny.gguf"),
            sha256: sha256.into(),
            bytes: 5,
            notes: String::new(),
            wants_gpu: false,
            streaming: false,
        }
    }

    #[test]
    fn downloads_verify_the_checksum() {
        let dir = temp_dir("catalog").join("models");
        let server = Server::start(vec![Reply::json("hello"), Reply::json("hellp")]);
        let good = tiny(
            &server,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
        );
        assert!(!installed(&dir, &good));
        let mut seen = (0, 0);
        let dest = download(&dir, &good, &mut |done, total| {
            seen = (done, total);
            true
        })
        .unwrap();
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "hello");
        assert_eq!(seen, (5, 5));
        assert!(installed(&dir, &good));
        // Already there: no request is made.
        download(&dir, &good, &mut |_, _| true).unwrap();
        assert_eq!(server.requests().len(), 1);

        std::fs::remove_file(&dest).unwrap();
        assert_eq!(
            download(&dir, &good, &mut |_, _| true).unwrap_err(),
            "Tiny arrived damaged. Download it again."
        );
        assert!(!dest.exists());
        assert!(sha256(&dest).unwrap_err().starts_with("Could not read"));
        let blocked = dest.parent().unwrap().join("blocked");
        std::fs::write(&blocked, "file").unwrap();
        assert!(download(&blocked.join("sub"), &good, &mut |_, _| true)
            .unwrap_err()
            .starts_with("Could not create"));
    }
}
