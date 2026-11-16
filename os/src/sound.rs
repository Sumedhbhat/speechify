//! Muting system audio while the user dictates.

/// Mutes or unmutes the default output device. Best effort: if it cannot be
/// done, dictation carries on unmuted.
pub fn mute(on: bool) {
    imp::mute(on);
}

#[cfg(windows)]
mod imp {
    use std::cell::RefCell;
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

    struct MutedOutput {
        volume: IAudioEndpointVolume,
        previous: bool,
        _apartment: crate::com::Apartment,
    }

    impl Drop for MutedOutput {
        fn drop(&mut self) {
            // Restore this endpoint even if the default device changed.
            let _ = unsafe { self.volume.SetMute(self.previous, std::ptr::null()) };
        }
    }

    thread_local! {
        static MUTED: RefCell<Option<MutedOutput>> = const { RefCell::new(None) };
    }

    pub fn mute(on: bool) {
        MUTED.with(|muted| {
            let mut muted = muted.borrow_mut();
            if !on {
                muted.take();
            } else if muted.is_none() {
                let Some(apartment) = crate::com::Apartment::new() else { return };
                let result: windows::core::Result<MutedOutput> = unsafe {
                    (|| {
                        let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
                        let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
                        let volume: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None)?;
                        let previous = volume.GetMute()?.as_bool();
                        volume.SetMute(true, std::ptr::null())?;
                        Ok(MutedOutput {
                            volume,
                            previous,
                            _apartment: apartment,
                        })
                    })()
                };
                *muted = result.ok();
            }
        });
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::cell::Cell;
    use std::process::Command;

    thread_local! {
        static PREVIOUS: Cell<Option<bool>> = const { Cell::new(None) };
    }

    fn set(on: bool) -> bool {
        Command::new("osascript")
            .args(["-e", &format!("set volume output muted {on}")])
            .status()
            .is_ok_and(|status| status.success())
    }

    pub fn mute(on: bool) {
        PREVIOUS.with(|previous| {
            if !on {
                if let Some(value) = previous.take() {
                    set(value);
                }
            } else if previous.get().is_none() {
                let Ok(output) = Command::new("osascript")
                    .args(["-e", "output muted of (get volume settings)"])
                    .output()
                else {
                    return;
                };
                if output.status.success() {
                    if let Some(value) = super::parse_boolean(&String::from_utf8_lossy(&output.stdout)) {
                        if set(true) {
                            previous.set(Some(value));
                        }
                    }
                }
            }
        });
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    use std::cell::Cell;
    use std::process::Command;

    #[derive(Clone, Copy)]
    enum Backend {
        PipeWire,
        Pulse,
    }

    thread_local! {
        static PREVIOUS: Cell<Option<(Backend, bool)>> = const { Cell::new(None) };
    }

    fn set(backend: Backend, on: bool) -> bool {
        let value = if on { "1" } else { "0" };
        let mut command = match backend {
            Backend::PipeWire => {
                let mut command = Command::new("wpctl");
                command.args(["set-mute", "@DEFAULT_AUDIO_SINK@", value]);
                command
            }
            Backend::Pulse => {
                let mut command = Command::new("pactl");
                command.args(["set-sink-mute", "@DEFAULT_SINK@", value]);
                command
            }
        };
        command.status().is_ok_and(|status| status.success())
    }

    fn current(backend: Backend) -> Option<bool> {
        let output = match backend {
            Backend::PipeWire => Command::new("wpctl")
                .args(["get-volume", "@DEFAULT_AUDIO_SINK@"])
                .output(),
            Backend::Pulse => Command::new("pactl")
                .env("LC_ALL", "C")
                .args(["get-sink-mute", "@DEFAULT_SINK@"])
                .output(),
        }
        .ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        match backend {
            Backend::PipeWire => super::pipewire_muted(&text),
            Backend::Pulse => super::parse_boolean(text.trim().strip_prefix("Mute:")?),
        }
    }

    pub fn mute(on: bool) {
        PREVIOUS.with(|previous| {
            if !on {
                if let Some((backend, value)) = previous.take() {
                    set(backend, value);
                }
            } else if previous.get().is_none() {
                for backend in [Backend::PipeWire, Backend::Pulse] {
                    if let Some(value) = current(backend) {
                        if set(backend, true) {
                            previous.set(Some((backend, value)));
                            break;
                        }
                    }
                }
            }
        });
    }
}

#[cfg(any(not(windows), test))]
fn parse_boolean(value: &str) -> Option<bool> {
    match value.trim() {
        "true" | "yes" => Some(true),
        "false" | "no" => Some(false),
        _ => None,
    }
}

#[cfg(any(not(any(windows, target_os = "macos")), test))]
fn pipewire_muted(value: &str) -> Option<bool> {
    let mut words = value.trim().strip_prefix("Volume:")?.split_whitespace();
    words.next()?.parse::<f32>().ok()?;
    match (words.next(), words.next()) {
        (Some("[MUTED]"), None) => Some(true),
        (None, None) => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mute_queries_preserve_muted_and_unmuted_states() {
        assert_eq!(parse_boolean("true\n"), Some(true));
        assert_eq!(parse_boolean("false"), Some(false));
        assert_eq!(parse_boolean(" yes"), Some(true));
        assert_eq!(parse_boolean("no"), Some(false));
        assert_eq!(parse_boolean(""), None);
        assert_eq!(parse_boolean("error"), None);
        assert_eq!(pipewire_muted("Volume: 0.30 [MUTED]\n"), Some(true));
        assert_eq!(pipewire_muted("Volume: 1.00\n"), Some(false));
        assert_eq!(pipewire_muted("Volume: error"), None);
        assert_eq!(pipewire_muted("Volume: 1.0 unexpected"), None);
        assert_eq!(pipewire_muted("error"), None);
    }
}
