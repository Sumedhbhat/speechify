use crate::{config::Result, ffi::*};
use libloading::Library;
use std::{
    ffi::{c_char, c_void, CStr, CString},
    path::Path,
    ptr,
};
// Resolve the complete C API once at the library boundary. An incomplete
// installation cannot make a model ready and later fail during dictation.
macro_rules! runtime_api {
    ($($name:ident: $signature:ty),* $(,)?) => {
        struct Api { $($name: $signature),* }
        impl Api {
            unsafe fn load(library: &Library) -> Result<Self> {
                Ok(Self { $($name: resolve(library, concat!(stringify!($name), "\0").as_bytes())?),* })
            }
        }
    };
}
unsafe fn resolve<T: Copy>(library: &Library, name: &[u8]) -> Result<T> {
    library
        .get::<T>(name)
        .map(|symbol| *symbol)
        .map_err(|error| {
            format!(
                "Native runtime is incomplete ({}): {error}",
                String::from_utf8_lossy(&name[..name.len() - 1])
            )
        })
}
runtime_api! {
    transcribe_backend_init_params_init: unsafe extern "C" fn(*mut transcribe_backend_init_params),
    transcribe_init_backends_ex: unsafe extern "C" fn(*const transcribe_backend_init_params) -> i32,
    transcribe_model_load_params_init: unsafe extern "C" fn(*mut transcribe_model_load_params),
    transcribe_model_load_file: unsafe extern "C" fn(*const c_char, *const transcribe_model_load_params, *mut *mut c_void) -> i32,
    transcribe_session_params_init: unsafe extern "C" fn(*mut transcribe_session_params),
    transcribe_session_init: unsafe extern "C" fn(*mut c_void, *const transcribe_session_params, *mut *mut c_void) -> i32,
    transcribe_capabilities_init: unsafe extern "C" fn(*mut transcribe_capabilities),
    transcribe_model_get_capabilities: unsafe extern "C" fn(*mut c_void, *mut transcribe_capabilities) -> i32,
    transcribe_model_device: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    transcribe_device_info_init: unsafe extern "C" fn(*mut transcribe_device_info),
    transcribe_device_get_info: unsafe extern "C" fn(*mut c_void, *mut transcribe_device_info) -> i32,
    transcribe_run_params_init: unsafe extern "C" fn(*mut transcribe_run_params),
    transcribe_model_supports: unsafe extern "C" fn(*mut c_void, i32) -> bool,
    transcribe_whisper_run_ext_init: unsafe extern "C" fn(*mut transcribe_whisper_run_ext),
    transcribe_run: unsafe extern "C" fn(*mut c_void, *const f32, i32, *const transcribe_run_params) -> i32,
    transcribe_full_text: unsafe extern "C" fn(*mut c_void) -> *const c_char,
    transcribe_session_free: unsafe extern "C" fn(*mut c_void),
    transcribe_model_free: unsafe extern "C" fn(*mut c_void),
}
pub(crate) struct Runtime {
    api: Api,
    // Function pointers stay valid until the last model/session has been freed.
    _library: Library,
}
pub(crate) unsafe fn load_runtime(dir: &Path) -> Result<Runtime> {
    let filename = if cfg!(target_os = "windows") {
        "transcribe.dll"
    } else if cfg!(target_os = "macos") {
        "libtranscribe.dylib"
    } else {
        "libtranscribe.so"
    };
    let path = dir.join(filename);
    #[cfg(windows)]
    let lib = {
        use libloading::os::windows::{
            Library as WindowsLibrary, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,
        };
        let path = path
            .canonicalize()
            .map_err(|e| format!("Native library: {e}"))?;
        // ggml.dll and ggml-base.dll are siblings of the pinned runtime DLL.
        WindowsLibrary::load_with_flags(
            path,
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
        )
        .map(Library::from)
        .map_err(|e| format!("Native library: {e}"))?
    };
    #[cfg(not(windows))]
    let lib = Library::new(path).map_err(|e| format!("Native library: {e}"))?;
    verify(&lib)?;
    let api = Api::load(&lib)?;
    Ok(Runtime { api, _library: lib })
}
pub struct Native {
    runtime: Runtime,
    model: *mut c_void,
    session: *mut c_void,
    pub device: String,
    max_samples: usize,
    languages: Vec<String>,
}
impl Native {
    pub fn open(dir: &Path, path: &Path, device: &str, threads: i32) -> Result<Self> {
        unsafe {
            let runtime = load_runtime(dir)?;
            let mut init: transcribe_backend_init_params = std::mem::zeroed();
            (runtime.api.transcribe_backend_init_params_init)(&mut init);
            let artifact = CString::new(dir.to_string_lossy().as_bytes())
                .expect("the loaded native directory contains no NUL");
            init.artifact_dir = artifact.as_ptr();
            init.allowed_backends = if device == "cpu" { 1 } else { u32::MAX };
            check((runtime.api.transcribe_init_backends_ex)(&init))?;
            let mut params: transcribe_model_load_params = std::mem::zeroed();
            (runtime.api.transcribe_model_load_params_init)(&mut params);
            params.backend = match device {
                "cpu" => 1,
                "metal" => 2,
                "vulkan" => 3,
                "cuda" => 5,
                _ => 0,
            };
            let path =
                CString::new(path.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
            let mut model = ptr::null_mut();
            check((runtime.api.transcribe_model_load_file)(
                path.as_ptr(),
                &params,
                &mut model,
            ))?;
            let mut this = Self {
                runtime,
                model,
                session: ptr::null_mut(),
                device: device.into(),
                max_samples: 0,
                languages: vec![],
            };
            let mut session: transcribe_session_params = std::mem::zeroed();
            (this.runtime.api.transcribe_session_params_init)(&mut session);
            session.n_threads = threads;
            check((this.runtime.api.transcribe_session_init)(
                this.model,
                &session,
                &mut this.session,
            ))?;
            let mut caps: transcribe_capabilities = std::mem::zeroed();
            (this.runtime.api.transcribe_capabilities_init)(&mut caps);
            check((this.runtime.api.transcribe_model_get_capabilities)(
                this.model, &mut caps,
            ))?;
            this.max_samples = if caps.max_audio_ms > 0 {
                caps.max_audio_ms as usize * 16
            } else {
                16000 * 600
            };
            if !caps.languages.is_null() {
                for i in 0..caps.n_languages {
                    this.languages.push(
                        CStr::from_ptr(*caps.languages.add(i as usize))
                            .to_string_lossy()
                            .into_owned(),
                    );
                }
            }
            // Report the selected native backend rather than the requested auto choice.
            let dev = (this.runtime.api.transcribe_model_device)(this.model);
            if !dev.is_null() {
                let mut info: transcribe_device_info = std::mem::zeroed();
                (this.runtime.api.transcribe_device_info_init)(&mut info);
                check((this.runtime.api.transcribe_device_get_info)(
                    dev, &mut info,
                ))?;
                if !info.kind.is_null() {
                    this.device = CStr::from_ptr(info.kind).to_string_lossy().into_owned();
                }
            }
            Ok(this)
        }
    }
    pub fn run(&mut self, audio: &[f32], language: &str, prompt: &str) -> Result<String> {
        unsafe {
            let mut results = vec![];
            for chunk in audio.chunks(self.max_samples.max(16000)) {
                let mut p: transcribe_run_params = std::mem::zeroed();
                (self.runtime.api.transcribe_run_params_init)(&mut p);
                let lang = CString::new(language).map_err(|e| e.to_string())?;
                if self.languages.iter().any(|x| x == language) {
                    p.language = lang.as_ptr();
                }
                let supports =
                    |feature| (self.runtime.api.transcribe_model_supports)(self.model, feature);
                if supports(4) {
                    p.pnc = 2;
                }
                let text = if language == "en" {
                    format!("Hello. This is a clear, punctuated English transcript. Use proper capitalization and punctuation. Glossary: {prompt}.")
                } else {
                    prompt.into()
                };
                let text = CString::new(text).map_err(|e| e.to_string())?;
                let mut ext: transcribe_whisper_run_ext = std::mem::zeroed();
                if supports(0) {
                    (self.runtime.api.transcribe_whisper_run_ext_init)(&mut ext);
                    ext.initial_prompt = text.as_ptr();
                    p.family = &ext.ext;
                } else if supports(8) {
                    p.prompt = text.as_ptr();
                }
                let words: Vec<CString> = prompt
                    .split(',')
                    .map(|x| {
                        CString::new(x.trim()).expect("the complete prompt was validated above")
                    })
                    .collect();
                let pointers: Vec<_> = words.iter().map(|x| x.as_ptr()).collect();
                if supports(7) {
                    p.vocabulary = pointers.as_ptr();
                    p.n_vocabulary = pointers.len() as i32;
                }
                check((self.runtime.api.transcribe_run)(
                    self.session,
                    chunk.as_ptr(),
                    chunk.len() as i32,
                    &p,
                ))?;
                let out = (self.runtime.api.transcribe_full_text)(self.session);
                if !out.is_null() {
                    results.push(CStr::from_ptr(out).to_string_lossy().trim().to_owned());
                }
            }
            Ok(results.join(" "))
        }
    }
}
fn check(status: i32) -> Result<()> {
    if status == 0 {
        Ok(())
    } else {
        Err(format!("transcribe.cpp status {status}"))
    }
}
impl Drop for Native {
    fn drop(&mut self) {
        unsafe {
            if !self.session.is_null() {
                (self.runtime.api.transcribe_session_free)(self.session);
            }
            if !self.model.is_null() {
                (self.runtime.api.transcribe_model_free)(self.model);
            }
        }
    }
}
