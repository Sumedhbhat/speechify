//! Platform-independent dictation logic.
//!
//! Everything here runs headless: the app supplies audio, the focused
//! application and a way to paste.

pub mod asr;
pub mod audio;
pub mod catalog;
pub mod context;
pub mod diff;
pub mod format;
pub mod http;
pub mod job;
pub mod learn;
pub mod live;
pub mod llm;
pub mod native;
pub mod pipeline;
pub mod settings;
pub mod store;
pub mod teacher;
pub mod trace;

/// Errors are shown to the user as-is, so they are plain sentences.
pub type Result<T> = std::result::Result<T, String>;

pub(crate) fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(any(test, feature = "fakes"))]
pub mod testing;
