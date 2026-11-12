//! Operating-system adapters.
//!
//! Everything here talks to real devices or other applications, so none of
//! it can run in a headless test. It is kept as thin as possible: decisions
//! live in `speechify-core`, which is fully tested; this crate only does
//! what it is told. It deliberately has no dependency on the core so the
//! Windows and macOS code can be compile-checked from any machine.

#[cfg(windows)]
mod com;

pub mod capture;
pub mod focus;
pub mod keys;
pub mod sound;
pub mod typing;

/// True in a Wayland session, where an app can neither watch global key
/// state nor send keystrokes to other apps.
pub fn is_wayland() -> bool {
    cfg!(target_os = "linux")
        && std::env::var("XDG_SESSION_TYPE").is_ok_and(|kind| kind.eq_ignore_ascii_case("wayland"))
}
