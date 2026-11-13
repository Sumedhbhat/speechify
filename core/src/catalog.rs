//! The speech models the app can download, pinned by revision and checksum.

use crate::http::Http;
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
    notes: &'static str,
}

const PINS: &[Pin] = &[
    Pin {
        id: "whisper-large-v3-turbo",
        name: "Whisper large-v3 turbo",
        revision: "18ada752005f5fddef499256f9a458d0b37a203a",
        sha256: "257aa7ec9121c22c167d5e2c49e3e76b4ba0e6e27ebc13074eba0a80f8190220",
        bytes: 886_381_728,
        wants_gpu: true,
        notes: "Most accurate on Indian English, names and numbers. Fast on a graphics card.",
    },
    Pin {
        id: "parakeet-tdt-0.6b-v3",
        name: "Parakeet TDT 0.6B v3",
        revision: "d8d8c32cb2017c57e5307767343e97870f8a7f53",
        sha256: "5859f77944efcd8eafa23a6350731960b2b55b2203df51f319665c807d802cc7",
        bytes: 739_508_576,
        wants_gpu: false,
        notes: "Fastest. The best choice without a graphics card; a little weaker on names.",
    },
    Pin {
        id: "parakeet-tdt-0.6b-v2",
        name: "Parakeet TDT 0.6B v2",
        revision: "58239cf6d0ecb0af921e7936c1e5e0a8dba91dbb",
        sha256: "f0d0e99cebb6d3b83f1f7069b82b5d3c2e39a54545b0da039cb4bafd9c4e5caa",
        bytes: 729_574_912,
        wants_gpu: false,
        notes: "English-only Parakeet; slightly better than v3 on plain English.",
    },
    Pin {
        id: "whisper-large-v3",
        name: "Whisper large-v3",
        revision: "274ad1d1cf680b7ee53cc70f78461394cd3a344b",
        sha256: "2fa1a5f179f8a511a53e2108db270aa4af3ce08cd976af4180e2854666bb4ba3",
        bytes: 1_668_741_440,
        wants_gpu: true,
        notes: "The full-size model. Used as the nightly teacher; slow for live dictation.",
    },
    Pin {
        id: "whisper-small",
        name: "Whisper small",
        revision: "4afdc7942ecf1997f359deae48a63506b062eac4",
        sha256: "9b9c8811bbcc82a7766f0fb0925614bdacb0923b2cc630daeac17108b655b860",
        bytes: 269_751_136,
        wants_gpu: false,
        notes: "Small download for weak hardware. Noticeably less accurate.",
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

/// The models chosen for this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Plan {
    pub speech_model: &'static str,
    pub teacher_model: &'static str,
    /// Language-model cleanup: only worth its delay on a graphics card.
    pub rewrite: bool,
    /// One sentence for the user on why these were picked.
    pub reason: String,
}

/// Enough graphics memory for Whisper turbo plus a 4B language model.
const GPU_MEMORY_MB: u64 = 4000;

/// Picks models so that everyday dictation is always fast. A graphics card
/// runs Whisper turbo in well under a second; without one, Parakeet is the
/// only model that keeps up (about 2 s per dictation on a laptop CPU against
/// 35 s for Whisper turbo; see docs/evaluation.md), and the larger Whisper
/// does the nightly review where nobody is waiting.
pub fn plan(devices: &[Device]) -> Plan {
    let card = devices
        .iter()
        .filter(|device| device.class == DeviceClass::Gpu && device.memory_mb >= GPU_MEMORY_MB)
        .max_by_key(|device| device.memory_mb);
    match card {
        Some(card) => Plan {
            speech_model: "whisper-large-v3-turbo",
            teacher_model: "whisper-large-v3",
            rewrite: true,
            reason: format!(
                "Found {} with {} GB: using Whisper turbo on it",
                card.name,
                card.memory_mb / 1000
            ),
        },
        None => Plan {
            speech_model: "parakeet-tdt-0.6b-v3",
            teacher_model: "whisper-large-v3-turbo",
            rewrite: false,
            reason: "No graphics card with enough memory: using Parakeet, the fastest model on a CPU".into(),
        },
    }
}

/// On the very first start, before any settings exist, chooses the models
/// for this machine and saves them. Later starts keep the user's choices.
pub fn first_run(store: &crate::store::Store, devices: &[Device]) -> Result<Option<Plan>> {
    if store.get("settings").is_some() {
        return Ok(None);
    }
    let chosen = plan(devices);
    let mut settings = crate::settings::Settings::default();
    settings.speech.model = chosen.speech_model.into();
    settings.teacher.speech_model = chosen.teacher_model.into();
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
        assert_eq!(all.len(), 5);
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

        // The defaults in Settings must exist here.
        let settings = crate::settings::Settings::default();
        find(&settings.speech.model).unwrap();
        find(&settings.teacher.speech_model).unwrap();
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
    fn a_graphics_card_gets_whisper_and_a_cpu_gets_parakeet() {
        let cpu = device(DeviceClass::Cpu, 16_000, "AMD Ryzen 5 3500U");
        let igpu = device(DeviceClass::IntegratedGpu, 8000, "AMD Radeon Vega 8");
        let small = device(DeviceClass::Gpu, 2048, "GeForce MX150");
        let rtx = device(DeviceClass::Gpu, 8188, "NVIDIA GeForce RTX 4060 Laptop GPU");
        let bigger = device(DeviceClass::Gpu, 12_288, "NVIDIA GeForce RTX 4070");
        let laptop = plan(&[cpu.clone(), rtx.clone()]);
        assert_eq!(
            (laptop.speech_model, laptop.teacher_model, laptop.rewrite),
            ("whisper-large-v3-turbo", "whisper-large-v3", true)
        );
        assert_eq!(
            laptop.reason,
            "Found NVIDIA GeForce RTX 4060 Laptop GPU with 8 GB: using Whisper turbo on it"
        );
        assert!(plan(&[rtx, bigger]).reason.contains("RTX 4070"));
        // Integrated graphics, a small card or no card at all: Parakeet, no cleanup model.
        for machine in [vec![cpu.clone()], vec![cpu.clone(), igpu], vec![cpu, small], vec![]] {
            let chosen = plan(&machine);
            assert_eq!(
                (chosen.speech_model, chosen.teacher_model, chosen.rewrite),
                ("parakeet-tdt-0.6b-v3", "whisper-large-v3-turbo", false)
            );
            assert!(chosen.reason.starts_with("No graphics card"));
        }
        // Every model a plan names is in the catalog.
        for chosen in [laptop, plan(&[])] {
            assert!(find(chosen.speech_model).is_ok() && find(chosen.teacher_model).is_ok());
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
    fn the_first_start_chooses_models_and_later_starts_keep_them() {
        let store = crate::store::Store::memory();
        let chosen = first_run(&store, &[]).unwrap().unwrap();
        let settings = store.settings();
        assert_eq!(
            (settings.speech.model.as_str(), settings.rewrite.enabled),
            (chosen.speech_model, false)
        );
        assert_eq!(settings.teacher.speech_model, "whisper-large-v3-turbo");
        // The user changes their mind; a later start must not undo that.
        let mut changed = settings;
        changed.speech.model = "whisper-small".into();
        store.save_settings(&changed).unwrap();
        assert_eq!(
            first_run(&store, &[device(DeviceClass::Gpu, 8000, "RTX")]).unwrap(),
            None
        );
        assert_eq!(store.settings().speech.model, "whisper-small");
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
