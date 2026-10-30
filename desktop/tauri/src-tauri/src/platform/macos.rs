//! Native macos keyboard, focus and clipboard operations.
use crate::config::Result;
use std::collections::BTreeSet;
use std::{
    io::Write,
    process::{Command, Stdio},
};

pub use crate::mac_clipboard::snapshot_and_write;
use std::ffi::c_void;
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn CGEventSourceKeyState(state: i32, key: u16) -> bool;
    fn CGPreflightListenEventAccess() -> bool;
    fn AXIsProcessTrusted() -> bool;
    fn AXUIElementCreateSystemWide() -> *mut c_void;
    fn AXUIElementCopyAttributeValue(
        element: *mut c_void,
        attribute: *const c_void,
        value: *mut *mut c_void,
    ) -> i32;
    fn AXUIElementGetPid(element: *mut c_void, pid: *mut i32) -> i32;
    fn CGEventCreateKeyboardEvent(source: *const c_void, key: u16, down: bool) -> *mut c_void;
    fn CGEventSetFlags(event: *mut c_void, flags: u64);
    fn CGEventPost(tap: i32, event: *mut c_void);
}
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFStringCreateWithCString(
        allocator: *const c_void,
        text: *const i8,
        encoding: u32,
    ) -> *mut c_void;
    fn CFRelease(value: *const c_void);
    fn CFHash(value: *const c_void) -> usize;
}
pub struct Poller;
impl Poller {
    pub fn new() -> Result<Self> {
        unsafe {
            if !AXIsProcessTrusted() || !CGPreflightListenEventAccess() {
                return Err("Grant Accessibility and Input Monitoring to Speechify Tauri Candidate, then restart".into());
            }
        }
        Ok(Self)
    }
    pub fn down(&self) -> Result<BTreeSet<String>> {
        let mapping = mapping();
        let mut out = BTreeSet::new();
        for k in 0..128 {
            if unsafe { CGEventSourceKeyState(0, k) } {
                out.insert(
                    mapping
                        .iter()
                        .find(|(_, code)| *code == k)
                        .map(|(name, _)| name.clone())
                        .unwrap_or_else(|| format!("key-{k}")),
                );
            }
        }
        Ok(out)
    }
    pub fn focus(&self) -> Result<u64> {
        unsafe {
            let system = AXUIElementCreateSystemWide();
            let attr = CFStringCreateWithCString(
                std::ptr::null(),
                c"AXFocusedUIElement".as_ptr(),
                0x08000100,
            );
            let mut app = std::ptr::null_mut();
            let code = AXUIElementCopyAttributeValue(system, attr, &mut app);
            CFRelease(attr);
            CFRelease(system);
            if code != 0 || app.is_null() {
                return Err(
                    "Cannot identify focused application; check Accessibility permission".into(),
                );
            }
            let mut pid = 0;
            let code = AXUIElementGetPid(app, &mut pid);
            let focus_hash = CFHash(app);
            CFRelease(app);
            if code != 0 || pid <= 0 {
                return Err("No focused application".into());
            }
            Ok((pid as u64) << 32 ^ focus_hash as u64)
        }
    }
}
fn mapping() -> Vec<(String, u16)> {
    let letters = [
        ("a", 0),
        ("s", 1),
        ("d", 2),
        ("f", 3),
        ("h", 4),
        ("g", 5),
        ("z", 6),
        ("x", 7),
        ("c", 8),
        ("v", 9),
        ("b", 11),
        ("q", 12),
        ("w", 13),
        ("e", 14),
        ("r", 15),
        ("y", 16),
        ("t", 17),
        ("1", 18),
        ("2", 19),
        ("3", 20),
        ("4", 21),
        ("6", 22),
        ("5", 23),
        ("9", 25),
        ("7", 26),
        ("8", 28),
        ("0", 29),
        ("o", 31),
        ("u", 32),
        ("i", 34),
        ("p", 35),
        ("l", 37),
        ("j", 38),
        ("k", 40),
        ("n", 45),
        ("m", 46),
        ("enter", 36),
        ("tab", 48),
        ("space", 49),
        ("backspace", 51),
        ("esc", 53),
        ("cmd_r", 54),
        ("cmd_l", 55),
        ("shift_l", 56),
        ("caps_lock", 57),
        ("alt_l", 58),
        ("ctrl_l", 59),
        ("shift_r", 60),
        ("alt_r", 61),
        ("ctrl_r", 62),
        ("f1", 122),
        ("f2", 120),
        ("f3", 99),
        ("f4", 118),
        ("f5", 96),
        ("f6", 97),
        ("f7", 98),
        ("f8", 100),
        ("f9", 101),
        ("f10", 109),
        ("f11", 103),
        ("f12", 111),
        ("f13", 105),
        ("f14", 107),
        ("f15", 113),
        ("f16", 106),
        ("f17", 64),
        ("f18", 79),
        ("f19", 80),
        ("f20", 90),
        ("left", 123),
        ("right", 124),
        ("down", 125),
        ("up", 126),
    ];
    letters.into_iter().map(|(s, k)| (s.into(), k)).collect()
}
pub fn write_clipboard(text: &str) -> Result<()> {
    let mut c = Command::new("/usr/bin/pbcopy")
        .stdin(Stdio::piped())
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
    unsafe {
        for (key, down, flags) in [
            (55, true, 1 << 20),
            (9, true, 1 << 20),
            (9, false, 1 << 20),
            (55, false, 0),
        ] {
            let e = CGEventCreateKeyboardEvent(std::ptr::null(), key, down);
            if e.is_null() {
                return Err("Cannot create paste event".into());
            }
            CGEventSetFlags(e, flags);
            CGEventPost(0, e);
            CFRelease(e);
        }
    }
    Ok(())
}
