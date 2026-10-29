// A deterministic external native engine. Tests inspect the parameters the
// Speechify adapter sends across the C ABI, without downloading model weights.
use std::{ffi::{c_char, c_void, CStr, CString}, sync::atomic::{AtomicI32, Ordering}};
static BAD_ABI: AtomicI32 = AtomicI32::new(-1);
#[no_mangle]
pub extern "C" fn fixture_bad_abi(id: i32) { BAD_ABI.store(id, Ordering::Relaxed); }
macro_rules! layout { ($id:expr, $f:ident) => {
    match $id {
        0 => std::mem::$f::<transcribe_model_load_params>(),
        1 => std::mem::$f::<transcribe_session_params>(),
        2 => std::mem::$f::<transcribe_run_params>(),
        3 => std::mem::$f::<transcribe_stream_params>(),
        4 => std::mem::$f::<transcribe_capabilities>(),
        5 => std::mem::$f::<transcribe_timings>(),
        6 => std::mem::$f::<transcribe_segment>(),
        7 => std::mem::$f::<transcribe_word>(),
        8 => std::mem::$f::<transcribe_token>(),
        9 => std::mem::$f::<transcribe_stream_update>(),
        10 => std::mem::$f::<transcribe_stream_text>(),
        11 => std::mem::$f::<transcribe_session_limits>(),
        12 => std::mem::$f::<transcribe_ext>(),
        13 => std::mem::$f::<transcribe_device_info>(),
        14 => std::mem::$f::<transcribe_speaker_segment>(),
        15 => std::mem::$f::<transcribe_backend_init_params>(),
        _ => 0,
    }
} }
#[no_mangle]
pub extern "C" fn transcribe_abi_struct_size(id: i32) -> usize {
    if BAD_ABI.load(Ordering::Relaxed) == id { 0 } else { layout!(id, size_of) }
}
#[no_mangle]
pub extern "C" fn transcribe_abi_struct_align(id: i32) -> usize { layout!(id, align_of) }
macro_rules! init { ($name:ident,$t:ty) => {
    #[no_mangle] pub unsafe extern "C" fn $name(p: *mut $t) { p.write(std::mem::zeroed()); }
} }
init!(transcribe_backend_init_params_init,transcribe_backend_init_params);
init!(transcribe_model_load_params_init,transcribe_model_load_params);
init!(transcribe_session_params_init,transcribe_session_params);
init!(transcribe_capabilities_init,transcribe_capabilities);
init!(transcribe_device_info_init,transcribe_device_info);
init!(transcribe_run_params_init,transcribe_run_params);
init!(transcribe_whisper_run_ext_init,transcribe_whisper_run_ext);
#[no_mangle]
pub extern "C" fn transcribe_init_backends_ex(_: *const transcribe_backend_init_params) -> i32 { 0 }
struct Model { mode: String, backend: i32 }
struct Session { model: *mut Model, output: CString }
#[no_mangle]
pub unsafe extern "C" fn transcribe_model_load_file(path: *const c_char, p: *const transcribe_model_load_params, out: *mut *mut c_void) -> i32 {
    let mode = CStr::from_ptr(path).to_string_lossy().into_owned();
    if mode.contains("bad-load") || mode.contains("gpu-load-fails") && (*p).backend != 1 { return 21; }
    *out = Box::into_raw(Box::new(Model { mode, backend: (*p).backend })).cast(); 0
}
#[no_mangle]
pub unsafe extern "C" fn transcribe_session_init(model: *mut c_void, p: *const transcribe_session_params, out: *mut *mut c_void) -> i32 {
    if (*model.cast::<Model>()).mode.contains("bad-session") { return 22; }
    if (*p).n_threads < 0 { return 23; }
    *out = Box::into_raw(Box::new(Session { model: model.cast(), output: CString::new("").unwrap() })).cast(); 0
}
#[no_mangle]
pub unsafe extern "C" fn transcribe_model_get_capabilities(model: *mut c_void, out: *mut transcribe_capabilities) -> i32 {
    let m = &*model.cast::<Model>();
    if m.mode.contains("bad-caps") { return 24; }
    (*out).max_audio_ms = if m.mode.contains("unlimited") { 0 } else { 1000 };
    if !m.mode.contains("no-languages") {
        // The ABI owns these strings for the lifetime of the loaded library.
        static mut LANGUAGES: [*const c_char; 2] = [b"en\0".as_ptr().cast(), b"fr\0".as_ptr().cast()];
        (*out).languages = std::ptr::addr_of!(LANGUAGES).cast(); (*out).n_languages = 2;
    }
    0
}
#[no_mangle]
pub unsafe extern "C" fn transcribe_model_device(model: *mut c_void) -> *mut c_void {
    if (*model.cast::<Model>()).mode.contains("no-device") { std::ptr::null_mut() } else { model }
}
#[no_mangle]
pub unsafe extern "C" fn transcribe_device_get_info(model: *mut c_void, out: *mut transcribe_device_info) -> i32 {
    let m = &*model.cast::<Model>();
    if m.mode.contains("bad-device") { return 25; }
    if !m.mode.contains("no-kind") {
        (*out).kind = match m.backend { 2 => b"metal\0".as_ptr(), 3 => b"vulkan\0".as_ptr(), 5 => b"cuda\0".as_ptr(), _ => b"cpu\0".as_ptr() }.cast();
    }
    0
}
#[no_mangle]
pub unsafe extern "C" fn transcribe_model_supports(model: *mut c_void, feature: i32) -> bool {
    let m = &*model.cast::<Model>();
    match feature { 0 => m.mode.contains("whisper"), 4 => !m.mode.contains("plain"), 7 => m.mode.contains("vocab"), 8 => m.mode.contains("prompt"), _ => false }
}
unsafe fn string(p: *const c_char) -> String { if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().into_owned() } }
#[no_mangle]
pub unsafe extern "C" fn transcribe_run(session: *mut c_void, _: *const f32, n: i32, p: *const transcribe_run_params) -> i32 {
    let s = &mut *session.cast::<Session>(); let m = &*s.model;
    if m.mode.contains("bad-run") || m.mode.contains("gpu-run-fails") && m.backend != 1 { return 26; }
    let prompt = if !(*p).family.is_null() { string((*((*p).family.cast::<transcribe_whisper_run_ext>())).initial_prompt) } else { string((*p).prompt) };
    let vocab: Vec<_> = (0..(*p).n_vocabulary).map(|i|string(*(*p).vocabulary.add(i as usize))).collect();
    s.output = CString::new(format!("{n}|{}|{prompt}|{}|pnc={}", string((*p).language), vocab.join(","), (*p).pnc)).unwrap(); 0
}
#[no_mangle]
pub unsafe extern "C" fn transcribe_full_text(session: *mut c_void) -> *const c_char {
    let s = &*session.cast::<Session>();
    if (*s.model).mode.contains("empty-result") { std::ptr::null() } else { s.output.as_ptr() }
}
#[no_mangle]
pub unsafe extern "C" fn transcribe_session_free(p: *mut c_void) { drop(Box::from_raw(p.cast::<Session>())); }
#[no_mangle]
pub unsafe extern "C" fn transcribe_model_free(p: *mut c_void) { drop(Box::from_raw(p.cast::<Model>())); }
