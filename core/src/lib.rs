//! Platform-independent dictation logic.
//!
//! Everything here runs headless: the desktop shell only supplies audio,
//! key state, the focused application and a way to paste.

pub mod app;
pub mod asr;
pub mod audio;
pub mod catalog;
pub mod context;
pub mod diff;
pub mod format;
pub mod hotkey;
pub mod http;
pub mod insights;
pub mod learn;
pub mod llm;
pub mod native;
pub mod pipeline;
pub mod session;
pub mod settings;
pub mod store;
pub mod teacher;

/// Errors are shown to the user as-is, so they are plain sentences.
pub type Result<T> = std::result::Result<T, String>;

pub(crate) fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
pub(crate) mod testing;
