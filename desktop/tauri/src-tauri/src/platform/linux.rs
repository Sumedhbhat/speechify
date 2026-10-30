//! Native linux keyboard, focus and clipboard operations.
use crate::config::Result;
use std::collections::BTreeSet;
use std::{
    io::Write,
    process::{Command, Stdio},
};

use libloading::Library;
use std::ffi::{c_char, c_void, CString};
pub struct Poller {
    lib: Option<Library>,
    evdev: Option<crate::evdev::Keyboard>,
    display: *mut c_void,
    mapping: Vec<(String, u8)>,
}
impl Poller {
    pub fn new() -> Result<Self> {
        if std::env::var_os("WAYLAND_DISPLAY").is_some()
            || std::env::var("XDG_SESSION_TYPE").is_ok_and(|s| s == "wayland")
        {
            return Ok(Self {
                lib: None,
                display: std::ptr::null_mut(),
                mapping: vec![],
                evdev: Some(crate::evdev::Keyboard::open()?),
            });
        }
        unsafe {
            let lib = Library::new("libX11.so.6").map_err(crate::config::err)?;
            let open = lib
                .get::<unsafe extern "C" fn(*const c_char) -> *mut c_void>(b"XOpenDisplay\0")
                .map_err(crate::config::err)?;
            let display = open(std::ptr::null());
            if display.is_null() {
                return Err("Cannot open X11 display".into());
            }
            let mut mapping = vec![];
            {
                let sym = lib
                    .get::<unsafe extern "C" fn(*const c_char) -> u64>(b"XStringToKeysym\0")
                    .map_err(crate::config::err)?;
                let code = lib
                    .get::<unsafe extern "C" fn(*mut c_void, u64) -> u8>(b"XKeysymToKeycode\0")
                    .map_err(crate::config::err)?;
                for (name, key) in key_names() {
                    mapping.push((
                        name,
                        code(display, sym(CString::new(key).unwrap().as_ptr())),
                    ));
                }
            }
            Ok(Self {
                lib: Some(lib),
                evdev: None,
                display,
                mapping,
            })
        }
    }
    pub fn down(&self) -> Result<BTreeSet<String>> {
        if let Some(k) = &self.evdev {
            return k.down();
        }
        unsafe {
            let f = self
                .lib
                .as_ref()
                .unwrap()
                .get::<unsafe extern "C" fn(*mut c_void, *mut c_char) -> i32>(b"XQueryKeymap\0")
                .map_err(crate::config::err)?;
            let mut map = [0u8; 32];
            f(self.display, map.as_mut_ptr().cast());
            let mut out: BTreeSet<String> = self
                .mapping
                .iter()
                .filter(|(_, c)| *c > 0 && map[*c as usize / 8] & (1 << (*c % 8)) != 0)
                .map(|(n, _)| n.clone())
                .collect();
            for code in 8..256usize {
                if map[code / 8] & (1 << (code % 8)) != 0
                    && !self.mapping.iter().any(|(_, c)| *c as usize == code)
                {
                    out.insert(format!("key-{code}"));
                }
            }
            Ok(out)
        }
    }
    pub fn focus(&self) -> Result<u64> {
        if self.evdev.is_some() {
            return Err("Wayland does not expose a safe focused target; transcript will be saved without pasting".into());
        }
        unsafe {
            let f = self
                .lib
                .as_ref()
                .unwrap()
                .get::<unsafe extern "C" fn(*mut c_void, *mut u64, *mut i32) -> i32>(
                    b"XGetInputFocus\0",
                )
                .map_err(crate::config::err)?;
            let (mut window, mut revert) = (0, 0);
            f(self.display, &mut window, &mut revert);
            if window <= 1 {
                return Err("No safe focused X11 target".into());
            }
            Ok(window)
        }
    }
}
impl Drop for Poller {
    fn drop(&mut self) {
        if self.lib.is_none() {
            return;
        }
        unsafe {
            if let Ok(f) = self
                .lib
                .as_ref()
                .unwrap()
                .get::<unsafe extern "C" fn(*mut c_void)>(b"XCloseDisplay\0")
            {
                f(self.display);
            }
        }
    }
}
fn key_names() -> Vec<(String, String)> {
    let mut v = vec![
        ("ctrl_l", "Control_L"),
        ("ctrl_r", "Control_R"),
        ("alt_l", "Alt_L"),
        ("alt_r", "Alt_R"),
        ("shift_l", "Shift_L"),
        ("shift_r", "Shift_R"),
        ("cmd_l", "Super_L"),
        ("cmd_r", "Super_R"),
        ("esc", "Escape"),
        ("scroll_lock", "Scroll_Lock"),
        ("caps_lock", "Caps_Lock"),
        ("space", "space"),
        ("enter", "Return"),
        ("backspace", "BackSpace"),
        ("tab", "Tab"),
        ("left", "Left"),
        ("right", "Right"),
        ("up", "Up"),
        ("down", "Down"),
    ]
    .into_iter()
    .map(|(a, b)| (a.into(), b.into()))
    .collect::<Vec<_>>();
    for c in 'a'..='z' {
        v.push((c.to_string(), c.to_string()));
    }
    for c in '0'..='9' {
        v.push((c.to_string(), c.to_string()));
    }
    for i in 1..=24 {
        v.push((format!("f{i}"), format!("F{i}")));
    }
    v
}
pub fn write_clipboard(text: &str) -> Result<()> {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        let mut c = Command::new("wl-copy")
            .stdin(Stdio::piped())
            .spawn()
            .map_err(crate::config::err)?;
        c.stdin
            .take()
            .unwrap()
            .write_all(text.as_bytes())
            .map_err(crate::config::err)?;
        if !c.wait().map_err(crate::config::err)?.success() {
            return Err("Wayland clipboard write failed".into());
        }
        return Ok(());
    }
    let mut c = Command::new("xclip")
        .args(["-selection", "clipboard", "-i", "-loops", "0"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(crate::config::err)?;
    c.stdin
        .take()
        .unwrap()
        .write_all(text.as_bytes())
        .map_err(crate::config::err)?;
    if !c.wait().map_err(crate::config::err)?.success() {
        return Err("Clipboard write failed".into());
    }
    Ok(())
}
pub fn paste_keys() -> Result<()> {
    let status = Command::new("xdotool")
        .args(["key", "--clearmodifiers", "ctrl+v"])
        .status()
        .map_err(|e| format!("Install xdotool: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("X11 paste failed".into())
    }
}
