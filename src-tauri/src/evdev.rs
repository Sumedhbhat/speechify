//! Optional Linux input source. Never changes permissions, creates uinput devices,
//! or opens anything during fixture/diagnostic commands.
use crate::config::Result;
use std::{
    cell::RefCell,
    collections::BTreeSet,
    fs::{File, OpenOptions},
    io::Read,
    os::unix::fs::OpenOptionsExt,
    path::Path,
};
#[cfg(test)]
#[path = "../tests/unit/evdev.rs"]
mod behavior;
pub struct Keyboard {
    devices: RefCell<Vec<(File, BTreeSet<String>)>>,
}
impl Keyboard {
    pub fn open() -> Result<Self> {
        Self::open_at(Path::new("/dev/input/by-id"))
    }
    fn open_at(root: &Path) -> Result<Self> {
        let mut devices = vec![];
        let entries=std::fs::read_dir(root).map_err(|_|"Wayland recording requires explicit read permission on your keyboard's /dev/input/by-id/*-event-kbd device. No permissions are changed automatically.")?;
        for entry in entries.flatten() {
            if !entry.file_name().to_string_lossy().ends_with("-event-kbd") {
                continue;
            }
            if let Ok(file) = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
                .open(entry.path())
            {
                devices.push((file, BTreeSet::new()));
            }
        }
        if devices.is_empty() {
            return Err("No readable keyboard event device. Ask your system administrator for narrowly scoped evdev access, or use X11. Automatic paste remains unavailable on Wayland.".into());
        }
        Ok(Self {
            devices: RefCell::new(devices),
        })
    }
    pub fn down(&self) -> Result<BTreeSet<String>> {
        let mut result = BTreeSet::new();
        for (file, held) in self.devices.borrow_mut().iter_mut() {
            loop {
                let mut event = std::mem::MaybeUninit::<libc::input_event>::zeroed();
                let bytes = unsafe {
                    std::slice::from_raw_parts_mut(
                        event.as_mut_ptr().cast::<u8>(),
                        std::mem::size_of::<libc::input_event>(),
                    )
                };
                match file.read(bytes) {
                    Ok(0) => {
                        return Err("Keyboard disconnected; reconnect and restart Speechify".into())
                    }
                    Ok(n) if n == bytes.len() => {
                        let e = unsafe { event.assume_init() };
                        if e.type_ == 0 && e.code == 3 {
                            held.clear();
                            return Err("Keyboard event stream overflowed; hold cancelled, restart Speechify".into());
                        }
                        if e.type_ == 1 {
                            let name = key_name(e.code);
                            if e.value == 0 {
                                held.remove(&name);
                            } else if e.value == 1 {
                                held.insert(name);
                            }
                        }
                    }
                    Ok(_) => return Err("Incomplete keyboard event".into()),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e) => return Err(e.to_string()),
                }
            }
            result.extend(held.iter().cloned());
        }
        Ok(result)
    }
}
fn key_name(code: u16) -> String {
    let name = match code {
        1 => "esc",
        14 => "backspace",
        15 => "tab",
        28 => "enter",
        29 => "ctrl_l",
        42 => "shift_l",
        54 => "shift_r",
        56 => "alt_l",
        57 => "space",
        58 => "caps_lock",
        70 => "scroll_lock",
        97 => "ctrl_r",
        100 => "alt_r",
        103 => "up",
        105 => "left",
        106 => "right",
        108 => "down",
        125 => "cmd_l",
        126 => "cmd_r",
        _ => "",
    };
    if !name.is_empty() {
        return name.into();
    }
    for (start, text) in [
        (2, "1234567890"),
        (16, "qwertyuiop"),
        (30, "asdfghjkl"),
        (44, "zxcvbnm"),
    ] {
        if code >= start && (code - start) < text.len() as u16 {
            return text
                .chars()
                .nth((code - start) as usize)
                .unwrap()
                .to_string();
        }
    }
    if (59..=68).contains(&code) {
        return format!("f{}", code - 58);
    }
    if code == 87 || code == 88 {
        return format!("f{}", code - 76);
    }
    if (183..=194).contains(&code) {
        return format!("f{}", code - 170);
    }
    format!("key-{code}")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maps_physical_keys() {
        assert_eq!(key_name(97), "ctrl_r");
        assert_eq!(key_name(46), "c");
        assert_eq!(key_name(191), "f21");
    }
}
