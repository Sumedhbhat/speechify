//! Real Win32 controls on the disposable coverage desktop.
use super::{read, write_clipboard, CLIPBOARD};
use crate::{behavior_tests::wait_until, platform::Poller, replacement};
use std::{
    ffi::c_void,
    time::{Duration, Instant},
};
type WindowHandle = *mut c_void;

#[repr(C)]
struct Message {
    window: WindowHandle,
    message: u32,
    w: usize,
    l: isize,
    time: u32,
    point: [i32; 2],
    private: u32,
}
#[link(name = "user32")]
extern "system" {
    fn CreateWindowExW(
        ex: u32,
        class: *const u16,
        title: *const u16,
        style: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        parent: WindowHandle,
        menu: WindowHandle,
        instance: WindowHandle,
        param: WindowHandle,
    ) -> WindowHandle;
    fn DestroyWindow(window: WindowHandle) -> i32;
    fn SendMessageW(window: WindowHandle, message: u32, w: usize, l: isize) -> isize;
    fn SetForegroundWindow(window: WindowHandle) -> i32;
    fn SetFocus(window: WindowHandle) -> WindowHandle;
    fn GetForegroundWindow() -> WindowHandle;
    fn GetWindowThreadProcessId(window: WindowHandle, process: *mut u32) -> u32;
    fn AttachThreadInput(first: u32, second: u32, attach: i32) -> i32;
    fn PeekMessageW(
        message: *mut Message,
        window: WindowHandle,
        first: u32,
        last: u32,
        remove: u32,
    ) -> i32;
    fn TranslateMessage(message: *const Message) -> i32;
    fn DispatchMessageW(message: *const Message) -> isize;
}
#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentThreadId() -> u32;
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
struct Window(WindowHandle);
impl Window {
    fn new(class: &str) -> Self {
        unsafe {
            let handle = CreateWindowExW(
                0,
                wide(class).as_ptr(),
                wide("Speechify behavior test").as_ptr(),
                0x10cf0044,
                20,
                20,
                400,
                200,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
            assert!(!handle.is_null());
            Self(handle)
        }
    }
    fn id(&self) -> u64 {
        self.0 as u64
    }
    fn send(&self, message: u32, w: usize, l: isize) -> isize {
        unsafe { SendMessageW(self.0, message, w, l) }
    }
    fn set(&self, text: &str) {
        assert_ne!(self.send(0x000c, 0, wide(text).as_ptr() as isize), 0);
    }
    fn select(&self, start: usize, end: usize) {
        self.send(0x00b1, start, end as isize);
    }
    fn insert(&self, text: &str) {
        self.send(0x00c2, 1, wide(text).as_ptr() as isize);
    }
    fn text(&self) -> String {
        let mut text = vec![0u16; self.send(0x000e, 0, 0) as usize + 1];
        let length = self.send(0x000d, text.len(), text.as_mut_ptr() as isize) as usize;
        String::from_utf16(&text[..length]).unwrap()
    }
    fn pump(&self) {
        unsafe {
            let mut message = std::mem::MaybeUninit::<Message>::zeroed();
            while PeekMessageW(message.as_mut_ptr(), std::ptr::null_mut(), 0, 0, 1) != 0 {
                TranslateMessage(message.as_ptr());
                DispatchMessageW(message.as_ptr());
            }
        }
    }
    fn focus(&self) {
        unsafe {
            let foreground = GetWindowThreadProcessId(GetForegroundWindow(), std::ptr::null_mut());
            let current = GetCurrentThreadId();
            let attached = foreground != 0
                && foreground != current
                && AttachThreadInput(current, foreground, 1) != 0;
            SetForegroundWindow(self.0);
            SetFocus(self.0);
            if attached {
                AttachThreadInput(current, foreground, 0);
            }
        }
        self.pump();
    }
    fn pasted(&self) -> replacement::Receipt {
        self.set("A old Z");
        self.select(2, 5);
        let receipt = replacement::capture(self.id(), "🦀 new").unwrap();
        self.insert("🦀 new");
        assert_eq!(self.text(), "A 🦀 new Z");
        receipt
    }
}
impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.0);
        }
    }
}

#[test]
#[ignore = "Uses native windows and paste events; requires a disposable desktop"]
fn standard_edit_corrections_preserve_surrounding_text_support_undo_and_refuse_stale_targets() {
    assert_eq!(
        std::env::var("SPEECHIFY_TEST_DISPOSABLE_DESKTOP").as_deref(),
        Ok("1")
    );
    let _desktop = CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner());
    let window = Window::new("Edit");
    let receipt = window.pasted();
    replacement::replace(&receipt, "GitHub").unwrap();
    assert_eq!(window.text(), "A GitHub Z");
    assert_ne!(window.send(0x00c7, 0, 0), 0); // EM_UNDO
    assert_eq!(window.text(), "A 🦀 new Z");
    let receipt = window.pasted();
    window.select(0, 0);
    assert!(replacement::replace(&receipt, "must not replace").is_err());
    assert_eq!(window.text(), "A 🦀 new Z");
    let receipt = window.pasted();
    window.set("The user typed something else");
    assert!(replacement::replace(&receipt, "must not replace").is_err());
    assert_eq!(window.text(), "The user typed something else");
    let mut expired = window.pasted();
    expired.created = Instant::now() - Duration::from_secs(121);
    assert!(replacement::replace(&expired, "must not replace")
        .unwrap_err()
        .contains("expired"));
    assert_eq!(window.text(), "A 🦀 new Z");
    let other = Window::new("STATIC");
    assert!(replacement::capture(other.id(), "new").is_none());
    window.send(0x00c5, 2_000_000, 0); // EM_SETLIMITTEXT
    window.set(&"x".repeat(1_000_001));
    assert!(replacement::capture(window.id(), "new").is_none());
    let closed = window.pasted();
    drop(window);
    assert!(replacement::replace(&closed, "must not replace").is_err());
}

#[test]
#[ignore = "Uses native windows and paste events; requires a disposable desktop"]
fn native_paste_inserts_into_the_focused_edit_and_restores_the_previous_clipboard() {
    assert_eq!(
        std::env::var("SPEECHIFY_TEST_DISPOSABLE_DESKTOP").as_deref(),
        Ok("1")
    );
    let _desktop = CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner());
    let window = Window::new("Edit");
    window.set("A old Z");
    window.select(2, 5);
    window.focus();
    let poller = Poller::new().unwrap();
    assert_eq!(poller.focus().unwrap(), window.id());
    assert!(poller.down().unwrap().is_empty());
    write_clipboard("Earlier copy 🦀").unwrap();
    let receipt = crate::platform::paste(window.id(), "new dictation")
        .unwrap()
        .unwrap();
    wait_until(|| {
        window.pump();
        window.text() == "A new dictation Z"
    });
    wait_until(|| read() == "Earlier copy 🦀");
    replacement::replace(&receipt, "GitHub").unwrap();
    assert_eq!(window.text(), "A GitHub Z");
}
