//! On-device recognition through the transcribe.cpp runtime.
//!
//! The runtime is a prebuilt library (see scripts/fetch-native.py) loaded at
//! run time, so the app needs no C++ toolchain to build and one binary can
//! use CPU, Vulkan, Metal or CUDA depending on the machine.

use crate::asr::{prompt, Hint, Recognizer};
use crate::Result;
use libloading::Library;
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::Path;
use std::ptr;
use std::sync::OnceLock;

// The structs below mirror transcribe.h 0.4.1. Each ends in spare space:
// the library's `*_init` writes its own `struct_size` and defaults, so a
// newer runtime with extra trailing fields still fits. `check_abi` verifies
// at load time that the fields used here exist and the spare room suffices.
type Spare = [u64; 8];

#[repr(C)]
struct BackendInit {
    struct_size: u64,
    artifact_dir: *const c_char,
    allowed_backends: u32,
    spare: Spare,
}

#[repr(C)]
struct DeviceInfo {
    struct_size: u64,
    name: *const c_char,
    description: *const c_char,
    kind: *const c_char,
    device_id: *const c_char,
    memory_total: u64,
    memory_free: u64,
    device_type: i32,
    spare: Spare,
}

#[repr(C)]
struct LoadParams {
    struct_size: u64,
    backend: i32,
    device: *mut c_void,
    no_mmap: bool,
    spare: Spare,
}

#[repr(C)]
struct SessionParams {
    struct_size: u64,
    n_threads: i32,
    kv_type: i32,
    n_ctx: i32,
    spare: Spare,
}

#[repr(C)]
struct RunParams {
    struct_size: u64,
    task: i32,
    timestamps: i32,
    pnc: i32,
    itn: i32,
    diarize: i32,
    language: *const c_char,
    target_language: *const c_char,
    keep_special_tags: bool,
    family: *const c_void,
    spec_k_drafts: i32,
    vocabulary: *const *const c_char,
    n_vocabulary: i32,
    prompt: *const c_char,
    prefix: *const c_char,
    spare: Spare,
}

#[repr(C)]
struct Capabilities {
    struct_size: u64,
    native_sample_rate: i32,
    n_languages: i32,
    languages: *const *const c_char,
    max_timestamp_kind: i32,
    supports_language_detect: bool,
    supports_translate: bool,
    supports_streaming: bool,
    supports_spec_decode: bool,
    max_audio_ms: i64,
    n_translate_target_languages: i32,
    translate_target_languages: *const *const c_char,
    spare: Spare,
}

const FEATURE_PNC: i32 = 4;
const FEATURE_VOCABULARY: i32 = 7;
const FEATURE_CONTEXT_PROMPT: i32 = 8;
const PNC_ON: i32 = 2;
const MASK_CPU: u32 = 1;
const MASK_ALL: u32 = u32::MAX;
const SPARE_BYTES: usize = std::mem::size_of::<Spare>();

macro_rules! api {
    ($($name:ident: $signature:ty),* $(,)?) => {
        struct Api { $($name: $signature),* }
        impl Api {
            unsafe fn load(library: &Library) -> Result<Api> {
                Ok(Api { $($name: *library.get::<$signature>(concat!(stringify!($name), "\0").as_bytes())
                    .map_err(|_| format!("The speech runtime is missing {}; reinstall Speechify", stringify!($name)))?),* })
            }
        }
    };
}

api! {
    transcribe_abi_struct_size: unsafe extern "C" fn(i32) -> usize,
    transcribe_backend_init_params_init: unsafe extern "C" fn(*mut BackendInit),
    transcribe_init_backends_ex: unsafe extern "C" fn(*const BackendInit) -> i32,
    transcribe_model_load_params_init: unsafe extern "C" fn(*mut LoadParams),
    transcribe_model_load_file: unsafe extern "C" fn(*const c_char, *const LoadParams, *mut *mut c_void) -> i32,
    transcribe_session_params_init: unsafe extern "C" fn(*mut SessionParams),
    transcribe_session_init: unsafe extern "C" fn(*mut c_void, *const SessionParams, *mut *mut c_void) -> i32,
    transcribe_capabilities_init: unsafe extern "C" fn(*mut Capabilities),
    transcribe_model_get_capabilities: unsafe extern "C" fn(*mut c_void, *mut Capabilities) -> i32,
    transcribe_model_device: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    transcribe_device_info_init: unsafe extern "C" fn(*mut DeviceInfo),
    transcribe_device_get_info: unsafe extern "C" fn(*mut c_void, *mut DeviceInfo) -> i32,
    transcribe_run_params_init: unsafe extern "C" fn(*mut RunParams),
    transcribe_model_supports: unsafe extern "C" fn(*mut c_void, i32) -> bool,
    transcribe_run: unsafe extern "C" fn(*mut c_void, *const f32, i32, *const RunParams) -> i32,
    transcribe_full_text: unsafe extern "C" fn(*mut c_void) -> *const c_char,
    transcribe_session_free: unsafe extern "C" fn(*mut c_void),
    transcribe_model_free: unsafe extern "C" fn(*mut c_void),
}

struct Runtime {
    api: Api,
    /// The backend mask the process registered; it cannot change afterwards.
    mask: u32,
    // Keeps the function pointers valid for the life of the process.
    _library: Library,
}

// The runtime is only reached through `Engine`, which is used from one
// thread at a time; the library itself is thread-safe per session.
unsafe impl Send for Runtime {}
unsafe impl Sync for Runtime {}

static RUNTIME: OnceLock<std::result::Result<Runtime, String>> = OnceLock::new();

fn explain(status: i32) -> String {
    match status {
        3 => "The speech model file is missing; download it again".into(),
        4..=6 => "The speech model file is damaged or not supported; download it again".into(),
        7 => "Not enough memory for this speech model; pick a smaller one".into(),
        8 => "The graphics driver could not run the speech model; set the device to CPU in Settings".into(),
        10 => "This speech model does not support the chosen language".into(),
        17 => "That recording is too long for this speech model".into(),
        other => format!("The speech engine failed (code {other})"),
    }
}

fn check(status: i32) -> Result<()> {
    if status == 0 {
        Ok(())
    } else {
        Err(explain(status))
    }
}

fn c_string(text: &str) -> CString {
    CString::new(text.replace('\0', " ")).expect("NUL bytes were replaced")
}

#[cfg(target_os = "windows")]
const LIBRARY: &str = "transcribe.dll";
#[cfg(target_os = "macos")]
const LIBRARY: &str = "libtranscribe.dylib";
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const LIBRARY: &str = "libtranscribe.so";

/// A struct fits when the runtime's version is no larger than the space
/// reserved here and no smaller than the fields this file reads.
fn abi_fits(actual: usize, reserved: usize) -> bool {
    actual <= reserved && actual + SPARE_BYTES >= reserved
}

/// Reads a C string owned by the runtime; null reads as empty.
unsafe fn text(pointer: *const c_char) -> String {
    if pointer.is_null() {
        String::new()
    } else {
        CStr::from_ptr(pointer).to_string_lossy().into_owned()
    }
}

/// Virtual machines expose a fake Metal device that produces garbage.
fn trusted(device: &str) -> bool {
    !device.to_lowercase().contains("paravirtual")
}

unsafe fn check_abi(api: &Api) -> Result<()> {
    // (index in transcribe_abi_struct, bytes this file relies on, bytes reserved)
    let sizes = [
        (0, std::mem::size_of::<LoadParams>()),
        (1, std::mem::size_of::<SessionParams>()),
        (2, std::mem::size_of::<RunParams>()),
        (4, std::mem::size_of::<Capabilities>()),
        (13, std::mem::size_of::<DeviceInfo>()),
        (15, std::mem::size_of::<BackendInit>()),
    ];
    let fits = sizes
        .iter()
        .all(|(which, reserved)| abi_fits((api.transcribe_abi_struct_size)(*which), *reserved));
    fits.then_some(())
        .ok_or_else(|| "The speech runtime does not match this version of Speechify; reinstall it".into())
}

unsafe fn load_runtime(dir: &Path, mask: u32) -> Result<Runtime> {
    let path = dir.join(LIBRARY);
    if !path.exists() {
        return Err(format!(
            "The speech runtime is not installed (looked for {})",
            path.display()
        ));
    }
    #[cfg(windows)]
    let library = {
        use libloading::os::windows::{
            Library as WindowsLibrary, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,
        };
        // The ggml DLLs sit next to transcribe.dll and must be found from there.
        let full = path
            .canonicalize()
            .map_err(|e| format!("The speech runtime could not be opened: {e}"))?;
        WindowsLibrary::load_with_flags(
            full,
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
        )
        .map(Library::from)
        .map_err(|e| format!("The speech runtime could not be opened: {e}"))?
    };
    #[cfg(not(windows))]
    let library = Library::new(&path).map_err(|e| format!("The speech runtime could not be opened: {e}"))?;
    let api = Api::load(&library)?;
    check_abi(&api)?;
    Ok(Runtime {
        api,
        mask,
        _library: library,
    })
}

fn runtime<'a>(cache: &'a OnceLock<Result<Runtime>>, dir: &Path, mask: u32) -> Result<&'a Runtime> {
    if let Some(loaded) = cache.get() {
        return loaded.as_ref().map_err(Clone::clone);
    }
    // Loading and checking the ABI has no backend side effects. Only failures
    // after calling init_backends must be cached: its process-wide state is fixed.
    let loaded = unsafe { load_runtime(dir, mask)? };
    cache
        .get_or_init(|| unsafe { init_backends(loaded, dir) })
        .as_ref()
        .map_err(Clone::clone)
}

unsafe fn init_backends(runtime: Runtime, dir: &Path) -> Result<Runtime> {
    let api = &runtime.api;
    let mut init: BackendInit = std::mem::zeroed();
    (api.transcribe_backend_init_params_init)(&mut init);
    let artifacts = c_string(&dir.to_string_lossy());
    init.artifact_dir = artifacts.as_ptr();
    init.allowed_backends = runtime.mask;
    check((api.transcribe_init_backends_ex)(&init))?;
    Ok(runtime)
}

/// A loaded speech model.
pub struct Engine {
    runtime: &'static Runtime,
    model: *mut c_void,
    session: *mut c_void,
    name: String,
    device: String,
    max_samples: usize,
    languages: Vec<String>,
}

// An engine is owned by one worker thread at a time.
unsafe impl Send for Engine {}

impl Engine {
    /// Loads `model` using the runtime in `runtime_dir`. `device` is "auto",
    /// "cpu", "vulkan", "metal" or "cuda".
    pub fn open(runtime_dir: &Path, model: &Path, device: &str) -> Result<Engine> {
        Self::open_if(runtime_dir, model, device, trusted)
    }

    /// Like `open`, but falls back to the CPU when `usable` rejects the
    /// device the model landed on.
    fn open_if(runtime_dir: &Path, model: &Path, device: &str, usable: fn(&str) -> bool) -> Result<Engine> {
        let cpu_only = device == "cpu";
        let runtime = runtime(&RUNTIME, runtime_dir, if cpu_only { MASK_CPU } else { MASK_ALL })?;
        if !cpu_only && runtime.mask == MASK_CPU {
            // GPU backends were never registered in this process and cannot be added now.
            return Err("Restart Speechify to switch from CPU to the graphics card".into());
        }
        // TRANSCRIBE_BACKEND_* codes; anything else lets the runtime choose.
        const BACKENDS: &[(&str, i32)] = &[("cpu", 1), ("metal", 2), ("vulkan", 3), ("cuda", 5)];
        let backend = BACKENDS
            .iter()
            .find(|(name, _)| *name == device)
            .map_or(0, |(_, code)| *code);
        let engine = unsafe { Self::load(runtime, model, backend)? };
        if usable(&engine.device) {
            return Ok(engine);
        }
        drop(engine);
        unsafe { Self::load(runtime, model, 1) }
    }

    unsafe fn load(runtime: &'static Runtime, model_path: &Path, backend: i32) -> Result<Engine> {
        let api = &runtime.api;
        let mut params: LoadParams = std::mem::zeroed();
        (api.transcribe_model_load_params_init)(&mut params);
        params.backend = backend;
        let path = c_string(&model_path.to_string_lossy());
        let mut model = ptr::null_mut();
        check((api.transcribe_model_load_file)(path.as_ptr(), &params, &mut model))?;
        let name = model_path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut engine = Engine {
            runtime,
            model,
            session: ptr::null_mut(),
            name,
            device: "cpu".into(),
            max_samples: 0,
            languages: vec![],
        };

        let mut session: SessionParams = std::mem::zeroed();
        (api.transcribe_session_params_init)(&mut session);
        // More threads than physical cores slows CPU inference down.
        session.n_threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 8) as i32;
        check((api.transcribe_session_init)(
            engine.model,
            &session,
            &mut engine.session,
        ))?;

        let mut caps: Capabilities = std::mem::zeroed();
        (api.transcribe_capabilities_init)(&mut caps);
        check((api.transcribe_model_get_capabilities)(engine.model, &mut caps))?;
        // Models without a stated limit get ten minutes per run.
        let limit_ms = if caps.max_audio_ms > 0 { caps.max_audio_ms } else { 600_000 };
        engine.max_samples = limit_ms as usize * 16;
        let languages = if caps.languages.is_null() { 0 } else { caps.n_languages.max(0) as usize };
        for i in 0..languages {
            let code = CStr::from_ptr(*caps.languages.add(i));
            engine.languages.push(code.to_string_lossy().into_owned());
        }

        let device = (api.transcribe_model_device)(engine.model);
        let mut info: DeviceInfo = std::mem::zeroed();
        (api.transcribe_device_info_init)(&mut info);
        // A failed query leaves the text fields null, which read as empty.
        let _ = !device.is_null() && (api.transcribe_device_get_info)(device, &mut info) == 0;
        let label = format!("{} {}", text(info.kind), text(info.description));
        engine.device = Some(label.trim())
            .filter(|label| !label.is_empty())
            .unwrap_or("cpu")
            .to_string();
        Ok(engine)
    }

    /// Whether this model can use the recogniser's vocabulary hint.
    pub fn supports_vocabulary(&self) -> bool {
        unsafe { (self.runtime.api.transcribe_model_supports)(self.model, FEATURE_VOCABULARY) }
    }

    /// The device the model actually runs on, e.g. "vulkan NVIDIA GeForce RTX 4060".
    pub fn device(&self) -> &str {
        &self.device
    }
}

impl Recognizer for Engine {
    fn transcribe(&mut self, audio: &[f32], hint: &Hint) -> Result<String> {
        if !hint.language.is_empty()
            && !self.languages.is_empty()
            && !self.languages.contains(&hint.language.to_string())
        {
            return Err(explain(10));
        }
        let api = &self.runtime.api;
        let supports = |feature| unsafe { (api.transcribe_model_supports)(self.model, feature) };
        let language = c_string(hint.language);
        let context = c_string(&prompt(&Hint {
            vocabulary: &[],
            ..*hint
        }));
        let terms: Vec<CString> = hint.vocabulary.iter().map(|term| c_string(term)).collect();
        let pointers: Vec<*const c_char> = terms.iter().map(|term| term.as_ptr()).collect();
        let mut pieces = Vec::new();
        for chunk in audio.chunks(self.max_samples.max(16_000)) {
            unsafe {
                let mut params: RunParams = std::mem::zeroed();
                (api.transcribe_run_params_init)(&mut params);
                if self.languages.iter().any(|known| known == hint.language) {
                    params.language = language.as_ptr();
                }
                params.pnc = if supports(FEATURE_PNC) { PNC_ON } else { params.pnc };
                if supports(FEATURE_CONTEXT_PROMPT) && !context.as_bytes().is_empty() {
                    params.prompt = context.as_ptr();
                }
                if supports(FEATURE_VOCABULARY) && !pointers.is_empty() {
                    params.vocabulary = pointers.as_ptr();
                    params.n_vocabulary = pointers.len() as i32;
                }
                check((api.transcribe_run)(
                    self.session,
                    chunk.as_ptr(),
                    chunk.len() as i32,
                    &params,
                ))?;
                let text = (api.transcribe_full_text)(self.session);
                if !text.is_null() {
                    pieces.push(CStr::from_ptr(text).to_string_lossy().trim().to_string());
                }
            }
        }
        Ok(pieces.join(" ").trim().to_string())
    }

    fn name(&self) -> &str {
        &self.name
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        unsafe {
            if !self.session.is_null() {
                (self.runtime.api.transcribe_session_free)(self.session);
            }
            (self.runtime.api.transcribe_model_free)(self.model);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// The runtime and a tiny model, when the environment provides them
    /// (`scripts/test-native.sh` and CI do). Set SPEECHIFY_REQUIRE_NATIVE=1
    /// to fail instead of skipping.
    fn fixture() -> Option<(PathBuf, PathBuf)> {
        let found = std::env::var_os("SPEECHIFY_NATIVE_DIR")
            .zip(std::env::var_os("SPEECHIFY_TEST_MODEL"))
            .map(|(a, b)| (PathBuf::from(a), PathBuf::from(b)));
        assert!(
            found.is_some() || std::env::var_os("SPEECHIFY_REQUIRE_NATIVE").is_none(),
            "native fixture missing"
        );
        found
    }

    #[test]
    fn status_codes_read_like_sentences() {
        assert!(check(0).is_ok());
        for (code, word) in [
            (3, "missing"),
            (4, "damaged"),
            (7, "memory"),
            (8, "graphics driver"),
            (10, "language"),
            (17, "too long"),
            (99, "code 99"),
        ] {
            assert!(check(code).unwrap_err().contains(word), "{code}");
        }
        assert_eq!(c_string("a\0b").to_str().unwrap(), "a b");
        assert!(abi_fits(40, 40 + SPARE_BYTES) && abi_fits(48 + SPARE_BYTES, 48 + SPARE_BYTES));
        assert!(!abi_fits(200, 100) && !abi_fits(8, 16 + SPARE_BYTES));
        assert!(trusted("vulkan NVIDIA GeForce RTX 4060") && !trusted("metal Apple Paravirtual device"));
        assert_eq!(unsafe { text(ptr::null()) }, "");
    }

    #[test]
    fn transcribes_speech_on_the_cpu() {
        let Some((runtime, model)) = fixture() else { return };
        let mut engine = Engine::open(&runtime, &model, "cpu").unwrap();
        assert!(engine.device().starts_with("cpu"), "{}", engine.device());
        assert_eq!(engine.name(), model.file_stem().unwrap().to_string_lossy());
        assert!(engine.supports_vocabulary());
        let audio = crate::audio::decode_wav(include_bytes!("../tests/fixtures/jfk.wav")).unwrap();
        let terms = vec!["Americans".to_string()];
        let text = engine
            .transcribe(
                &audio,
                &Hint {
                    language: "en",
                    vocabulary: &terms,
                },
            )
            .unwrap()
            .to_lowercase();
        assert!(text.contains("ask not what your country can do for you"), "{text}");
        // Silence must not crash, and a second run reuses the session.
        engine.transcribe(&vec![0.0; 16_000], &Hint::default()).unwrap();
        // A device that cannot be trusted is swapped for the CPU.
        let fallback = Engine::open_if(&runtime, &model, "cpu", |_| false).unwrap();
        assert!(fallback.device().starts_with("cpu"));

        assert_eq!(
            Engine::open(&runtime, &model, "vulkan").err().unwrap(),
            "Restart Speechify to switch from CPU to the graphics card"
        );
        assert!(Engine::open(&runtime, &runtime.join("missing.gguf"), "cpu")
            .err()
            .unwrap()
            .contains("missing"));
        let junk = crate::testing::temp_dir("native").join("junk.gguf");
        std::fs::write(&junk, b"not a model").unwrap();
        assert!(Engine::open(&runtime, &junk, "cpu").is_err());
    }

    #[test]
    fn a_missing_runtime_is_reported() {
        let dir = crate::testing::temp_dir("no-runtime");
        let error = unsafe { load_runtime(&dir, MASK_CPU) }.err().unwrap();
        assert!(error.starts_with("The speech runtime is not installed"), "{error}");
        std::fs::write(dir.join(LIBRARY), b"junk").unwrap();
        let error = unsafe { load_runtime(&dir, MASK_CPU) }.err().unwrap();
        assert!(error.starts_with("The speech runtime could not be opened"), "{error}");
    }

    #[test]
    fn runtime_loading_recovers_before_backend_initialization() {
        let Some((dir, _)) = fixture() else { return };
        let missing = crate::testing::temp_dir("runtime-retry");
        let cache = OnceLock::new();
        assert!(runtime(&cache, &missing, MASK_CPU)
            .err()
            .unwrap()
            .contains("not installed"));
        assert_eq!(runtime(&cache, &dir, MASK_CPU).unwrap().mask, MASK_CPU);
        assert!(runtime(&cache, &missing, MASK_CPU).is_ok());
        let failed = OnceLock::new();
        failed.set(Err("backend initialization failed".into())).ok().unwrap();
        assert_eq!(
            runtime(&failed, &dir, MASK_CPU).err().unwrap(),
            "backend initialization failed"
        );
    }

    #[test]
    fn explicit_unsupported_languages_fail_without_transcription() {
        let Some((runtime, model)) = fixture() else { return };
        let mut engine = Engine::open(&runtime, &model, "cpu").unwrap();
        assert!(engine.languages.contains(&"en".into()));
        assert!(engine
            .transcribe(
                &[0.0; 16000],
                &Hint {
                    language: "hi",
                    vocabulary: &[]
                }
            )
            .unwrap_err()
            .contains("language"));
        engine.languages.clear();
        assert!(engine
            .transcribe(
                &[0.0; 16000],
                &Hint {
                    language: "en",
                    vocabulary: &[]
                }
            )
            .is_ok());
    }
}
