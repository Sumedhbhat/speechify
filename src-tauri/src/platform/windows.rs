//! Native windows keyboard, focus and clipboard operations.
use crate::config::Result;
use std::collections::BTreeSet;

use std::ffi::c_void;
#[link(name = "user32")]
extern "system" {
    fn GetAsyncKeyState(key: i32) -> i16;
    fn GetForegroundWindow() -> *mut c_void;
    fn GetWindowThreadProcessId(hwnd: *mut c_void, pid: *mut u32) -> u32;
    fn GetGUIThreadInfo(thread: u32, info: *mut GuiInfo) -> i32;
    fn OpenClipboard(owner: *mut c_void) -> i32;
    fn CreateWindowExW(
        ex: u32,
        class: *const u16,
        name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        parent: *mut c_void,
        menu: *mut c_void,
        instance: *mut c_void,
        param: *mut c_void,
    ) -> *mut c_void;
    fn DestroyWindow(window: *mut c_void) -> i32;
    fn CloseClipboard() -> i32;
    fn GetClipboardData(format: u32) -> *mut c_void;
    fn EmptyClipboard() -> i32;
    fn SetClipboardData(format: u32, data: *mut c_void) -> *mut c_void;
    fn keybd_event(key: u8, scan: u8, flags: u32, extra: usize);
    fn EnumClipboardFormats(format: u32) -> u32;
    fn GetClipboardSequenceNumber() -> u32;
    fn GetClipboardOwner() -> *mut c_void;
}
#[link(name = "kernel32")]
extern "system" {
    fn GlobalLock(h: *mut c_void) -> *mut c_void;
    fn GlobalUnlock(h: *mut c_void) -> i32;
    fn GlobalAlloc(flags: u32, bytes: usize) -> *mut c_void;
    fn GlobalFree(h: *mut c_void) -> *mut c_void;
    fn GlobalSize(h: *mut c_void) -> usize;
}
#[repr(C)]
struct GuiInfo {
    size: u32,
    flags: u32,
    active: *mut c_void,
    focus: *mut c_void,
    capture: *mut c_void,
    menu: *mut c_void,
    move_size: *mut c_void,
    caret: *mut c_void,
    rect: [i32; 4],
}
pub struct Poller;
impl Poller {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }
    pub fn down(&self) -> Result<BTreeSet<String>> {
        let mut map = vec![
            ("ctrl_l".into(), 0xA2),
            ("ctrl_r".into(), 0xA3),
            ("alt_l".into(), 0xA4),
            ("alt_r".into(), 0xA5),
            ("shift_l".into(), 0xA0),
            ("shift_r".into(), 0xA1),
            ("cmd_l".into(), 0x5B),
            ("cmd_r".into(), 0x5C),
            ("esc".into(), 27),
            ("space".into(), 32),
            ("enter".into(), 13),
            ("backspace".into(), 8),
            ("tab".into(), 9),
            ("caps_lock".into(), 0x14),
            ("scroll_lock".into(), 0x91),
            ("left".into(), 37),
            ("right".into(), 39),
            ("up".into(), 38),
            ("down".into(), 40),
        ];
        for c in 'a'..='z' {
            map.push((c.to_string(), c.to_ascii_uppercase() as i32));
        }
        for c in '0'..='9' {
            map.push((c.to_string(), c as i32));
        }
        for i in 1..=24 {
            map.push((format!("f{i}"), 0x6f + i));
        }
        let mut out = BTreeSet::new();
        for key in 1..256 {
            if [1, 2, 4, 5, 6, 0x10, 0x11, 0x12].contains(&key) {
                continue;
            }
            if unsafe { GetAsyncKeyState(key) < 0 } {
                out.insert(
                    map.iter()
                        .find(|(_, code)| *code == key)
                        .map(|(name, _)| name.clone())
                        .unwrap_or_else(|| format!("key-{key}")),
                );
            }
        }
        Ok(out)
    }
    pub fn focus(&self) -> Result<u64> {
        unsafe {
            let window = GetForegroundWindow();
            if window.is_null() {
                return Err("No foreground window".into());
            }
            let thread = GetWindowThreadProcessId(window, std::ptr::null_mut());
            let mut info: GuiInfo = std::mem::zeroed();
            info.size = std::mem::size_of::<GuiInfo>() as u32;
            if thread == 0
                || GetGUIThreadInfo(thread, &mut info) == 0
                || info.focus.is_null()
                || info.active != window
            {
                return Err(
                    "Cannot identify focused Windows control; transcript saved without paste"
                        .into(),
                );
            }
            Ok(info.focus as u64)
        }
    }
}
struct Clip {
    window: *mut c_void,
    locked: bool,
}
impl Clip {
    fn committed_generation(&mut self) -> Result<u32> {
        unsafe {
            // Closing commits synthesized formats and advances the sequence.
            // These queries do not require reopening the clipboard, so a
            // reader cannot make us discard the saved snapshot after writing.
            CloseClipboard();
            self.locked = false;
            let generation = GetClipboardSequenceNumber();
            // A newer copy either fails this owner check or advances the
            // restore's sequence guard. Read the sequence before the owner.
            if GetClipboardOwner() != self.window {
                return Err("Clipboard changed before paste; transcript saved".into());
            }
            Ok(generation)
        }
    }
}
impl Drop for Clip {
    fn drop(&mut self) {
        unsafe {
            if self.locked {
                CloseClipboard();
            }
            DestroyWindow(self.window);
        }
    }
}
fn open() -> Result<Clip> {
    unsafe {
        let class: Vec<u16> = "STATIC".encode_utf16().chain(Some(0)).collect();
        let window = CreateWindowExW(
            0,
            class.as_ptr(),
            std::ptr::null(),
            0,
            0,
            0,
            0,
            0,
            -3isize as _,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        if window.is_null() {
            return Err("Cannot create clipboard owner window".into());
        }
        if OpenClipboard(window) == 0 {
            DestroyWindow(window);
            Err("Clipboard is busy; retry".into())
        } else {
            Ok(Clip {
                window,
                locked: true,
            })
        }
    }
}
pub fn write_clipboard(text: &str) -> Result<()> {
    let data: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    let _guard = open()?;
    unsafe {
        let h = GlobalAlloc(2, data.len() * 2);
        if h.is_null() {
            return Err("Clipboard allocation failed".into());
        }
        let p = GlobalLock(h) as *mut u16;
        if p.is_null() {
            GlobalFree(h);
            return Err("Clipboard lock failed".into());
        }
        std::ptr::copy_nonoverlapping(data.as_ptr(), p, data.len());
        GlobalUnlock(h);
        EmptyClipboard();
        if SetClipboardData(13, h).is_null() {
            GlobalFree(h);
            return Err("Clipboard set failed".into());
        }
    }
    Ok(())
}
pub fn snapshot_and_write(text: &str) -> Result<impl FnOnce() -> Result<()> + Send> {
    let mut formats = Vec::new();
    let generation;
    {
        let mut guard = open()?;
        unsafe {
            let mut format = EnumClipboardFormats(0);
            let mut total = 0;
            while format != 0 {
                // Bitmap, palette and metafile handles cannot be cloned as HGLOBAL.
                if [2, 3, 9, 14, 0x80, 0x81, 0x82, 0x83, 0x8e].contains(&format) {
                    return Err("Clipboard contains an OS handle format that cannot be preserved; use manual copy".into());
                }
                let h = GetClipboardData(format);
                let size = GlobalSize(h);
                total += size;
                if size == 0 || total > 32 * 1024 * 1024 {
                    return Err(
                        "Clipboard snapshot unavailable or exceeds 32 MB; use manual copy".into(),
                    );
                }
                let p = GlobalLock(h) as *const u8;
                if p.is_null() {
                    return Err("Cannot snapshot clipboard; use manual copy".into());
                }
                let bytes = std::slice::from_raw_parts(p, size).to_vec();
                GlobalUnlock(h);
                formats.push((format, bytes));
                format = EnumClipboardFormats(format);
            }
            let bytes: Vec<u8> = text
                .encode_utf16()
                .chain(Some(0))
                .flat_map(u16::to_ne_bytes)
                .collect();
            let h = allocate(&bytes)?;
            if EmptyClipboard() == 0 {
                GlobalFree(h);
                return Err("Clipboard clear failed".into());
            }
            if SetClipboardData(13, h).is_null() {
                GlobalFree(h);
                return Err("Clipboard set failed".into());
            }
            generation = guard.committed_generation()?;
        }
    }
    Ok(move || {
        let _guard = open()?;
        unsafe {
            // Sequence check and restore share the clipboard lock: no check/write race.
            if GetClipboardSequenceNumber() != generation {
                return Ok(());
            }
            let mut allocated = Vec::new();
            for (format, bytes) in formats {
                match allocate(&bytes) {
                    Ok(h) => allocated.push((format, h)),
                    Err(e) => {
                        for (_, h) in allocated {
                            GlobalFree(h);
                        }
                        return Err(e);
                    }
                }
            }
            if EmptyClipboard() == 0 {
                for (_, h) in allocated {
                    GlobalFree(h);
                }
                return Err("Clipboard restore busy".into());
            }
            let mut failed = false;
            for (format, h) in allocated {
                if SetClipboardData(format, h).is_null() {
                    GlobalFree(h);
                    failed = true;
                }
            }
            if failed {
                return Err("Clipboard restore failed".into());
            }
        }
        Ok(())
    })
}
unsafe fn allocate(bytes: &[u8]) -> Result<*mut c_void> {
    let h = GlobalAlloc(2, bytes.len());
    if h.is_null() {
        return Err("Clipboard allocation failed".into());
    }
    let p = GlobalLock(h) as *mut u8;
    if p.is_null() {
        GlobalFree(h);
        return Err("Clipboard lock failed".into());
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
    GlobalUnlock(h);
    Ok(h)
}
pub fn paste_keys() -> Result<()> {
    unsafe {
        keybd_event(0x11, 0, 0, 0);
        keybd_event(0x56, 0, 0, 0);
        keybd_event(0x56, 0, 2, 0);
        keybd_event(0x11, 0, 2, 0);
    }
    Ok(())
}
