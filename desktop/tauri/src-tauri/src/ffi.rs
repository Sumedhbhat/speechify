// Generated from transcribe.cpp 0.3.1; PUBLIC_HEADER_HASH 57b1af43650f195d
#![allow(non_camel_case_types, dead_code)]
#[repr(C)]
pub struct transcribe_ext {
    pub size: usize,
    pub kind: u32,
}
#[repr(C)]
pub struct transcribe_backend_init_params {
    pub struct_size: usize,
    pub artifact_dir: *const std::ffi::c_char,
    pub allowed_backends: u32,
}
#[repr(C)]
pub struct transcribe_device_info {
    pub struct_size: usize,
    pub name: *const std::ffi::c_char,
    pub description: *const std::ffi::c_char,
    pub kind: *const std::ffi::c_char,
    pub device_id: *const std::ffi::c_char,
    pub memory_total: usize,
    pub memory_free: usize,
    pub device_type: i32,
}
#[repr(C)]
pub struct transcribe_model_load_params {
    pub struct_size: usize,
    pub backend: i32,
    pub device: *mut std::ffi::c_void,
}
#[repr(C)]
pub struct transcribe_session_params {
    pub struct_size: usize,
    pub n_threads: i32,
    pub kv_type: i32,
    pub n_ctx: i32,
}
#[repr(C)]
pub struct transcribe_run_params {
    pub struct_size: usize,
    pub task: i32,
    pub timestamps: i32,
    pub pnc: i32,
    pub itn: i32,
    pub diarize: i32,
    pub language: *const std::ffi::c_char,
    pub target_language: *const std::ffi::c_char,
    pub keep_special_tags: bool,
    pub family: *const transcribe_ext,
    pub spec_k_drafts: i32,
    pub vocabulary: *const *const std::ffi::c_char,
    pub n_vocabulary: i32,
    pub prompt: *const std::ffi::c_char,
    pub prefix: *const std::ffi::c_char,
}
#[repr(C)]
pub struct transcribe_capabilities {
    pub struct_size: usize,
    pub native_sample_rate: i32,
    pub n_languages: i32,
    pub languages: *const *const std::ffi::c_char,
    pub max_timestamp_kind: i32,
    pub supports_language_detect: bool,
    pub supports_translate: bool,
    pub supports_streaming: bool,
    pub supports_spec_decode: bool,
    pub max_audio_ms: i64,
    pub n_translate_target_languages: i32,
    pub translate_target_languages: *const *const std::ffi::c_char,
}
#[repr(C)]
pub struct transcribe_session_limits {
    pub struct_size: usize,
    pub effective_n_ctx: i32,
    pub effective_max_audio_ms: i64,
    pub max_kv_bytes: i64,
}
#[repr(C)]
pub struct transcribe_stream_params {
    pub struct_size: usize,
    pub family: *const transcribe_ext,
    pub commit_policy: i32,
    pub stable_prefix_agreement_n: u32,
}
#[repr(C)]
pub struct transcribe_stream_update {
    pub struct_size: usize,
    pub result_changed: bool,
    pub is_final: bool,
    pub revision: i32,
    pub input_received_ms: i64,
    pub audio_committed_ms: i64,
    pub buffered_ms: i64,
    pub committed_changed: bool,
    pub tentative_changed: bool,
}
#[repr(C)]
pub struct transcribe_stream_text {
    pub struct_size: usize,
    pub full_text: *const std::ffi::c_char,
    pub full_text_bytes: usize,
    pub committed_text: *const std::ffi::c_char,
    pub committed_text_bytes: usize,
    pub tentative_text: *const std::ffi::c_char,
    pub tentative_text_bytes: usize,
    pub raw_tentative_start_bytes: usize,
}
#[repr(C)]
pub struct transcribe_timings {
    pub struct_size: usize,
    pub load_ms: f32,
    pub mel_ms: f32,
    pub encode_ms: f32,
    pub decode_ms: f32,
}
#[repr(C)]
pub struct transcribe_segment {
    pub struct_size: usize,
    pub t0_ms: i64,
    pub t1_ms: i64,
    pub first_word: i32,
    pub n_words: i32,
    pub first_token: i32,
    pub n_tokens: i32,
    pub text: *const std::ffi::c_char,
    pub speaker_id: i32,
}
#[repr(C)]
pub struct transcribe_word {
    pub struct_size: usize,
    pub t0_ms: i64,
    pub t1_ms: i64,
    pub seg_index: i32,
    pub first_token: i32,
    pub n_tokens: i32,
    pub text: *const std::ffi::c_char,
}
#[repr(C)]
pub struct transcribe_token {
    pub struct_size: usize,
    pub id: i32,
    pub p: f32,
    pub t0_ms: i64,
    pub t1_ms: i64,
    pub seg_index: i32,
    pub word_index: i32,
    pub text: *const std::ffi::c_char,
}
#[repr(C)]
pub struct transcribe_speaker_segment {
    pub struct_size: usize,
    pub t0_ms: i64,
    pub t1_ms: i64,
    pub speaker_id: i32,
    pub p: f32,
}
#[repr(C)]
pub struct transcribe_moonshine_streaming_stream_ext {
    pub ext: transcribe_ext,
    pub min_decode_interval_ms: i32,
}
#[repr(C)]
pub struct transcribe_parakeet_stream_ext {
    pub ext: transcribe_ext,
    pub att_context_right: i32,
}
#[repr(C)]
pub struct transcribe_parakeet_buffered_stream_ext {
    pub ext: transcribe_ext,
    pub left_ms: i32,
    pub chunk_ms: i32,
    pub right_ms: i32,
}
#[repr(C)]
pub struct transcribe_sortformer_stream_ext {
    pub ext: transcribe_ext,
    pub preset: i32,
}
#[repr(C)]
pub struct transcribe_voxtral_realtime_stream_ext {
    pub ext: transcribe_ext,
    pub num_delay_tokens: i32,
    pub min_decode_interval_ms: i32,
}
#[repr(C)]
pub struct transcribe_whisper_run_ext {
    pub ext: transcribe_ext,
    pub initial_prompt: *const std::ffi::c_char,
    pub prompt_tokens: *const i32,
    pub n_prompt_tokens: usize,
    pub prompt_condition: i32,
    pub condition_on_prev_tokens: bool,
    pub max_prev_context_tokens: i32,
    pub temperature: f32,
    pub temperature_inc: f32,
    pub compression_ratio_thold: f32,
    pub logprob_thold: f32,
    pub no_speech_thold: f32,
    pub seed: u32,
    pub max_initial_timestamp: f32,
}
#[repr(C)]
pub struct transcribe_whisper_chunk_trace {
    pub struct_size: usize,
    pub t0_ms: i64,
    pub t1_ms: i64,
    pub temperature_used: f32,
    pub compression_ratio: f32,
    pub avg_logprob: f32,
    pub no_speech_prob: f32,
    pub no_speech_triggered: bool,
    pub n_fallbacks: i32,
}
pub fn verify(lib: &libloading::Library) -> Result<(), String> {
    unsafe {
        let size: libloading::Symbol<unsafe extern "C" fn(i32) -> usize> = lib
            .get(b"transcribe_abi_struct_size\0")
            .map_err(|e| e.to_string())?;
        let align: libloading::Symbol<unsafe extern "C" fn(i32) -> usize> = lib
            .get(b"transcribe_abi_struct_align\0")
            .map_err(|e| e.to_string())?;
        if size(12) != std::mem::size_of::<transcribe_ext>()
            || align(12) != std::mem::align_of::<transcribe_ext>()
        {
            return Err("native ABI mismatch: transcribe_ext".into());
        }
        if size(15) != std::mem::size_of::<transcribe_backend_init_params>()
            || align(15) != std::mem::align_of::<transcribe_backend_init_params>()
        {
            return Err("native ABI mismatch: transcribe_backend_init_params".into());
        }
        if size(13) != std::mem::size_of::<transcribe_device_info>()
            || align(13) != std::mem::align_of::<transcribe_device_info>()
        {
            return Err("native ABI mismatch: transcribe_device_info".into());
        }
        if size(0) != std::mem::size_of::<transcribe_model_load_params>()
            || align(0) != std::mem::align_of::<transcribe_model_load_params>()
        {
            return Err("native ABI mismatch: transcribe_model_load_params".into());
        }
        if size(1) != std::mem::size_of::<transcribe_session_params>()
            || align(1) != std::mem::align_of::<transcribe_session_params>()
        {
            return Err("native ABI mismatch: transcribe_session_params".into());
        }
        if size(2) != std::mem::size_of::<transcribe_run_params>()
            || align(2) != std::mem::align_of::<transcribe_run_params>()
        {
            return Err("native ABI mismatch: transcribe_run_params".into());
        }
        if size(4) != std::mem::size_of::<transcribe_capabilities>()
            || align(4) != std::mem::align_of::<transcribe_capabilities>()
        {
            return Err("native ABI mismatch: transcribe_capabilities".into());
        }
        if size(11) != std::mem::size_of::<transcribe_session_limits>()
            || align(11) != std::mem::align_of::<transcribe_session_limits>()
        {
            return Err("native ABI mismatch: transcribe_session_limits".into());
        }
        if size(3) != std::mem::size_of::<transcribe_stream_params>()
            || align(3) != std::mem::align_of::<transcribe_stream_params>()
        {
            return Err("native ABI mismatch: transcribe_stream_params".into());
        }
        if size(9) != std::mem::size_of::<transcribe_stream_update>()
            || align(9) != std::mem::align_of::<transcribe_stream_update>()
        {
            return Err("native ABI mismatch: transcribe_stream_update".into());
        }
        if size(10) != std::mem::size_of::<transcribe_stream_text>()
            || align(10) != std::mem::align_of::<transcribe_stream_text>()
        {
            return Err("native ABI mismatch: transcribe_stream_text".into());
        }
        if size(5) != std::mem::size_of::<transcribe_timings>()
            || align(5) != std::mem::align_of::<transcribe_timings>()
        {
            return Err("native ABI mismatch: transcribe_timings".into());
        }
        if size(6) != std::mem::size_of::<transcribe_segment>()
            || align(6) != std::mem::align_of::<transcribe_segment>()
        {
            return Err("native ABI mismatch: transcribe_segment".into());
        }
        if size(7) != std::mem::size_of::<transcribe_word>()
            || align(7) != std::mem::align_of::<transcribe_word>()
        {
            return Err("native ABI mismatch: transcribe_word".into());
        }
        if size(8) != std::mem::size_of::<transcribe_token>()
            || align(8) != std::mem::align_of::<transcribe_token>()
        {
            return Err("native ABI mismatch: transcribe_token".into());
        }
        if size(14) != std::mem::size_of::<transcribe_speaker_segment>()
            || align(14) != std::mem::align_of::<transcribe_speaker_segment>()
        {
            return Err("native ABI mismatch: transcribe_speaker_segment".into());
        }
    }
    Ok(())
}
