//! Platform-specific setup guidance for the CLI and settings window.
use serde_json::{json, Value};

pub fn diagnostics() -> Value {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var("XDG_SESSION_TYPE").is_ok_and(|s| s == "wayland");
    let session = if cfg!(target_os = "linux") {
        if wayland {
            "Wayland"
        } else {
            "X11"
        }
    } else {
        "native"
    };
    let guidance = if cfg!(target_os = "macos") {
        "Grant Microphone, Accessibility and Input Monitoring to Speechify in System Settings. Restart after grants. Use a stable signing identity."
    } else if cfg!(windows) {
        "Enable microphone access for desktop applications. Windows UIPI blocks pasting into elevated applications from a non-elevated app."
    } else if wayland {
        "Record in app works without global-key grants; use Copy text afterward. Global hold-to-talk requires explicitly granted read access to /dev/input/by-id/*-event-kbd. Safe target discovery/paste is unavailable: transcripts are saved for manual copy (wl-clipboard required). No input privileges or uinput devices are created automatically. Use X11 for automatic paste."
    } else {
        "X11 requires DISPLAY, libX11, xclip and xdotool. Microphone permission is controlled by PipeWire/PulseAudio/ALSA. Clipboard snapshots retain supported MIME formats up to 128 KiB each; unsupported formats refuse automatic paste."
    };
    json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "session": session,
        "guidance": guidance,
    })
}
