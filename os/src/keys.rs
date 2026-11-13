//! Which keys are held down right now.

#[cfg(not(target_os = "linux"))]
use device_query::DeviceState;
use std::marker::PhantomData;
use std::rc::Rc;
#[cfg(target_os = "linux")]
use x11::DeviceState;

/// Reads global key state by polling, which works without installing a
/// keyboard hook and can never miss a key-up.
pub struct Keyboard {
    state: Option<DeviceState>,
    _thread: PhantomData<Rc<()>>,
}

impl Default for Keyboard {
    fn default() -> Self {
        Self::new()
    }
}

impl Keyboard {
    pub fn new() -> Keyboard {
        // Not available in a Wayland session or without an X display.
        let state = if crate::is_wayland() { None } else { DeviceState::checked_new() };
        Keyboard {
            state,
            _thread: PhantomData,
        }
    }

    pub fn available(&self) -> bool {
        self.state.is_some()
    }

    /// The keys currently held, by the names the shortcut settings use
    /// ("Ctrl", "Meta", "Space", "A", "F5"). Left and right modifiers are
    /// one key. Keys with no name here are reported as "Other".
    pub fn pressed(&self) -> Vec<String> {
        let Some(state) = &self.state else {
            return vec![];
        };
        #[cfg(not(target_os = "linux"))]
        {
            state
                .query_keymap()
                .iter()
                .map(|key| key_name(&format!("{key:?}")))
                .collect()
        }
        #[cfg(target_os = "linux")]
        {
            state.pressed()
        }
    }
}

fn key_name(code: &str) -> String {
    match code {
        "LControl" | "RControl" | "Control_L" | "Control_R" => "Ctrl".into(),
        "LShift" | "RShift" | "Shift_L" | "Shift_R" => "Shift".into(),
        "LAlt" | "RAlt" | "LOption" | "ROption" | "Alt_L" | "Alt_R" | "ISO_Level3_Shift" => "Alt".into(),
        "LMeta" | "RMeta" | "Command" | "RCommand" | "Super_L" | "Super_R" | "Meta_L" | "Meta_R" => "Meta".into(),
        "space" => "Space".into(),
        "Return" => "Enter".into(),
        "Caps_Lock" => "CapsLock".into(),
        "Space" | "Escape" | "CapsLock" | "Enter" | "Tab" => code.into(),
        _ => {
            let digit = code
                .strip_prefix("Key")
                .filter(|rest| rest.len() == 1 && rest.as_bytes()[0].is_ascii_digit());
            let function = code
                .strip_prefix('F')
                .filter(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()));
            if let Some(digit) = digit {
                digit.into()
            } else if function.is_some() || (code.len() == 1 && code.as_bytes()[0].is_ascii_alphanumeric()) {
                code.to_ascii_uppercase()
            } else {
                "Other".into()
            }
        }
    }
}

// device_query 4.0.1's Linux query_keymap passes a pointer to a temporary
// array after its lifetime ends. Keep an owned buffer alive through XQueryKeymap
// instead. This connection is created, queried and closed on a single thread.
#[cfg(target_os = "linux")]
mod x11 {
    use std::ffi::{c_char, c_int, c_ulong, c_void, CStr};
    use std::ptr::NonNull;

    #[link(name = "X11")]
    extern "C" {
        fn XOpenDisplay(name: *const c_char) -> *mut c_void;
        fn XCloseDisplay(display: *mut c_void) -> c_int;
        fn XQueryKeymap(display: *mut c_void, keys: *mut c_char) -> c_int;
        fn XKeycodeToKeysym(display: *mut c_void, keycode: u8, index: c_int) -> c_ulong;
        fn XKeysymToString(symbol: c_ulong) -> *const c_char;
    }

    pub(super) struct DeviceState(NonNull<c_void>);

    impl DeviceState {
        pub(super) fn checked_new() -> Option<Self> {
            NonNull::new(unsafe { XOpenDisplay(std::ptr::null()) }).map(Self)
        }

        pub(super) fn pressed(&self) -> Vec<String> {
            let mut keys = [0 as c_char; 32];
            unsafe { XQueryKeymap(self.0.as_ptr(), keys.as_mut_ptr()) };
            let mut names = Vec::new();
            for code in 8..=255_u8 {
                if keys[code as usize / 8] as u8 & (1 << (code % 8)) == 0 {
                    continue;
                }
                let name = unsafe { XKeysymToString(XKeycodeToKeysym(self.0.as_ptr(), code, 0)) };
                names.push(if name.is_null() {
                    "Other".into()
                } else {
                    super::key_name(&unsafe { CStr::from_ptr(name) }.to_string_lossy())
                });
            }
            names
        }
    }

    impl Drop for DeviceState {
        fn drop(&mut self) {
            unsafe { XCloseDisplay(self.0.as_ptr()) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::key_name;

    #[test]
    fn key_codes_map_to_shortcut_names() {
        for (code, name) in [
            ("LControl", "Ctrl"),
            ("RControl", "Ctrl"),
            ("RShift", "Shift"),
            ("LAlt", "Alt"),
            ("ROption", "Alt"),
            ("LMeta", "Meta"),
            ("Command", "Meta"),
            ("Space", "Space"),
            ("Escape", "Escape"),
            ("CapsLock", "CapsLock"),
            ("Enter", "Enter"),
            ("Tab", "Tab"),
            ("A", "A"),
            ("Z", "Z"),
            ("Key0", "0"),
            ("Key9", "9"),
            ("F1", "F1"),
            ("F12", "F12"),
            ("Left", "Other"),
            ("Numpad1", "Other"),
            ("Keypad", "Other"),
            ("F", "F"),
            ("Fx", "Other"),
            ("Control_L", "Ctrl"),
            ("Shift_R", "Shift"),
            ("Super_L", "Meta"),
            ("Meta_R", "Meta"),
            ("Alt_R", "Alt"),
            ("ISO_Level3_Shift", "Alt"),
            ("Return", "Enter"),
            ("Caps_Lock", "CapsLock"),
            ("space", "Space"),
            ("a", "A"),
            ("0", "0"),
        ] {
            assert_eq!(key_name(code), name, "{code}");
        }
    }
}
