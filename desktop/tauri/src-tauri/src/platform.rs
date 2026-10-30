//! Native key state and target guards. Created only by a normal desktop launch.
use crate::config::Result;
#[cfg(all(test, target_os = "linux"))]
#[path = "../tests/unit/platform.rs"]
mod behavior;
#[cfg(all(test, not(target_os = "linux")))]
#[path = "../tests/unit/clipboard.rs"]
mod clipboard_behavior;
use std::{
    collections::BTreeSet,
    io::Write,
    process::{Command, Stdio},
};
#[cfg(target_os = "linux")]
mod os {
    use super::*;
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
}
#[cfg(target_os = "macos")]
mod os {
    use super::*;
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
                        "Cannot identify focused application; check Accessibility permission"
                            .into(),
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
}
#[cfg(windows)]
mod os {
    use super::*;
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
                Ok(Clip { window, locked: true })
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
                            "Clipboard snapshot unavailable or exceeds 32 MB; use manual copy"
                                .into(),
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
}
pub use os::{write_clipboard, Poller};
pub fn paste(target: u64, text: &str) -> Result<Option<crate::replacement::Receipt>> {
    let p = Poller::new()?;
    if p.focus()? != target {
        return Err("Focus changed; transcript saved without pasting".into());
    }
    if !p.down()?.is_empty() {
        return Err("Keys are still held; transcript saved without pasting".into());
    }
    let receipt = crate::replacement::capture(target, text);
    #[cfg(target_os = "linux")]
    crate::xclipboard::begin(text)?;
    #[cfg(not(target_os = "linux"))]
    let restore = os::snapshot_and_write(text)?;
    std::thread::sleep(std::time::Duration::from_millis(25));
    let result = (|| {
        if p.focus()? != target || !p.down()?.is_empty() {
            Err("Focus or held keys changed before paste; transcript saved".into())
        } else {
            os::paste_keys()
        }
    })();
    #[cfg(not(target_os = "linux"))]
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(400));
        let _ = restore();
    });
    result?;
    Ok(receipt)
}
