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
    fn keybd_event(key: u8, scan: u8, flags: u32, extra: usize);
    fn GetWindowLongPtrW(window: WindowHandle, index: i32) -> isize;
    fn SetWindowLongPtrW(window: WindowHandle, index: i32, value: isize) -> isize;
    fn CallWindowProcW(
        procedure: WindowProcedure,
        window: WindowHandle,
        message: u32,
        w: usize,
        l: isize,
    ) -> isize;
}
type WindowProcedure = unsafe extern "system" fn(WindowHandle, u32, usize, isize) -> isize;
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
                0x10cf00c4,
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

struct SelectionChange<'a> {
    window: &'a Window,
    state: Box<SelectionState>,
}
struct SelectionState {
    original: WindowProcedure,
    change: bool,
}
unsafe extern "system" fn change_during_selection(
    window: WindowHandle,
    message: u32,
    w: usize,
    l: isize,
) -> isize {
    let state = GetWindowLongPtrW(window, -21) as *mut SelectionState;
    let original = (*state).original;
    let change = message == 0x00b1 && (*state).change;
    if change {
        (*state).change = false;
    }
    let result = CallWindowProcW(original, window, message, w, l);
    if change {
        SendMessageW(
            window,
            0x000c,
            0,
            wide("The user typed during the guard").as_ptr() as isize,
        );
    }
    result
}
impl<'a> SelectionChange<'a> {
    fn new(window: &'a Window) -> Self {
        unsafe {
            let original =
                std::mem::transmute::<isize, WindowProcedure>(GetWindowLongPtrW(window.0, -4));
            let mut state = Box::new(SelectionState {
                original,
                change: true,
            });
            SetWindowLongPtrW(window.0, -21, state.as_mut() as *mut _ as isize);
            SetWindowLongPtrW(window.0, -4, change_during_selection as *const () as isize);
            Self { window, state }
        }
    }
}
impl Drop for SelectionChange<'_> {
    fn drop(&mut self) {
        unsafe {
            SetWindowLongPtrW(self.window.0, -4, self.state.original as *const () as isize);
            SetWindowLongPtrW(self.window.0, -21, 0);
        }
    }
}

#[test]
#[ignore = "Uses native windows and key events; requires a disposable desktop"]
fn native_key_state_includes_right_modifiers_function_keys_and_unmapped_keys() {
    assert_eq!(
        std::env::var("SPEECHIFY_TEST_DISPOSABLE_DESKTOP").as_deref(),
        Ok("1")
    );
    let _desktop = CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner());
    let window = Window::new("Edit");
    window.focus();
    struct HeldKeys;
    impl Drop for HeldKeys {
        fn drop(&mut self) {
            for key in [0xa3, 0x41, 0x87, 0xba] {
                unsafe {
                    keybd_event(key, 0, if key == 0xa3 { 3 } else { 2 }, 0);
                }
            }
        }
    }
    let held = HeldKeys;
    for key in [0xa3, 0x41, 0x87, 0xba] {
        unsafe {
            keybd_event(key, 0, if key == 0xa3 { 1 } else { 0 }, 0);
        }
    }
    let poller = Poller::new().unwrap();
    let expected = ["ctrl_r".into(), "a".into(), "f24".into(), "key-186".into()]
        .into_iter()
        .collect();
    wait_until(|| {
        window.pump();
        poller.down().unwrap() == expected
    });
    drop(held);
    wait_until(|| {
        window.pump();
        poller.down().unwrap().is_empty()
    });
    unsafe {
        SetFocus(std::ptr::null_mut());
    }
    assert!(poller.focus().is_err());
}

#[test]
#[ignore = "Uses native windows; requires a disposable desktop"]
fn correction_refuses_a_target_that_changes_after_the_range_guard() {
    assert_eq!(
        std::env::var("SPEECHIFY_TEST_DISPOSABLE_DESKTOP").as_deref(),
        Ok("1")
    );
    let _desktop = CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner());
    let window = Window::new("Edit");
    let receipt = window.pasted();
    {
        let _change = SelectionChange::new(&window);
        assert!(replacement::replace(&receipt, "must not overwrite the user").is_err());
    }
    assert_eq!(window.text(), "The user typed during the guard");
}

#[test]
#[ignore = "Uses native windows; requires a disposable desktop"]
fn correction_refuses_a_busy_target_without_waiting_for_it_to_resume() {
    assert_eq!(
        std::env::var("SPEECHIFY_TEST_DISPOSABLE_DESKTOP").as_deref(),
        Ok("1")
    );
    let _desktop = CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner());
    let (ready, receipt) = std::sync::mpsc::channel();
    let (resume, waiting) = std::sync::mpsc::channel();
    let owner = std::thread::spawn(move || {
        let window = Window::new("Edit");
        ready.send(window.pasted()).unwrap();
        let _ = waiting.recv_timeout(Duration::from_secs(5));
        assert_eq!(window.text(), "A 🦀 new Z");
    });
    let receipt = receipt.recv_timeout(Duration::from_secs(5)).unwrap();
    let result = replacement::replace(&receipt, "must not replace");
    let _ = resume.send(());
    owner.join().unwrap();
    assert!(result.is_err());
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
    window.send(0x000b, 0, 0); // WM_SETREDRAW: avoid painting the oversized fixture
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

#[path = "../../../tests/fixtures/windows-hooks.rs"]
mod os_hooks;

fn failure_child() -> bool {
    let name = std::thread::current().name().unwrap().to_owned();
    if std::env::var("SPEECHIFY_TEST_WINDOWS_FAILURE_CHILD").as_deref() == Ok(&name) {
        return true;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &name, "--include-ignored"])
        .env("SPEECHIFY_TEST_WINDOWS_FAILURE_CHILD", &name)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    false
}
unsafe extern "system" fn no_handle(_: *mut std::ffi::c_void) -> *mut std::ffi::c_void {
    std::ptr::null_mut()
}
unsafe extern "system" fn allocation_failed(_: u32, _: usize) -> *mut std::ffi::c_void {
    std::ptr::null_mut()
}
unsafe extern "system" fn clipboard_busy(_: *mut std::ffi::c_void) -> i32 {
    0
}
unsafe extern "system" fn clipboard_clear_failed() -> i32 {
    0
}
unsafe extern "system" fn clipboard_set_failed(
    _: u32,
    _: *mut std::ffi::c_void,
) -> *mut std::ffi::c_void {
    std::ptr::null_mut()
}
unsafe extern "system" fn clipboard_data_missing(_: u32) -> *mut std::ffi::c_void {
    std::ptr::null_mut()
}
unsafe extern "system" fn stable_clipboard_generation() -> u32 {
    100
}
unsafe extern "system" fn no_owner() -> *mut std::ffi::c_void {
    std::ptr::null_mut()
}
unsafe extern "system" fn no_job(_: *mut std::ffi::c_void, _: *const u16) -> *mut std::ffi::c_void {
    std::ptr::null_mut()
}
unsafe extern "system" fn no_job_configuration(
    _: *mut std::ffi::c_void,
    _: i32,
    _: *const std::ffi::c_void,
    _: u32,
) -> i32 {
    0
}
unsafe extern "system" fn no_assignment(_: *mut std::ffi::c_void, _: *mut std::ffi::c_void) -> i32 {
    0
}
unsafe extern "system" fn no_resume(_: *mut std::ffi::c_void) -> i32 {
    -1
}

#[cfg(test)]
#[test]
fn worker_startup_refuses_every_failed_ownership_step_without_starting_an_unsupervised_engine() {
    if !failure_child() {
        return;
    }
    for (symbol, replacement, message) in [
        ("CreateJobObjectW", no_job as *const (), "create"),
        (
            "SetInformationJobObject",
            no_job_configuration as *const (),
            "configure",
        ),
        (
            "AssignProcessToJobObject",
            no_assignment as *const (),
            "establish",
        ),
        ("NtResumeProcess", no_resume as *const (), "establish"),
    ] {
        let home = crate::behavior_tests::Home::new();
        let service = crate::behavior_tests::fake_service(&home, "");
        let hook = unsafe { os_hooks::Hook::new(symbol, replacement) };
        let error = service.preload().unwrap_err();
        drop(hook);
        assert!(error.contains(message), "{symbol}: {error}");
        assert_eq!(service.pid.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(service.snapshot()["state"]["ready"], false);
        assert!(!home.0.join("models/whisper-ft-test/started").exists());
        service.preload().unwrap();
        assert_eq!(service.snapshot()["state"]["ready"], true);
    }
}

#[test]
#[ignore = "Simulates clipboard OS failures in an isolated disposable desktop process"]
fn clipboard_failures_are_reported_and_later_writes_can_recover() {
    let _desktop = CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner());
    if !failure_child() {
        return;
    }
    for (symbol, replacement, message) in [
        ("OpenClipboard", clipboard_busy as *const (), "busy"),
        ("GlobalAlloc", allocation_failed as *const (), "allocation"),
        ("GlobalLock", no_handle as *const (), "lock"),
        ("SetClipboardData", clipboard_set_failed as *const (), "set"),
    ] {
        write_clipboard("Previous copy").unwrap();
        let hook = unsafe { os_hooks::Hook::new(symbol, replacement) };
        let error = write_clipboard("Cannot complete").unwrap_err();
        drop(hook);
        assert!(error.contains(message), "{symbol}: {error}");
        write_clipboard("Recovered café 🦀").unwrap();
        assert_eq!(read(), "Recovered café 🦀");
    }
    for (symbol, replacement, message) in [
        (
            "GetClipboardData",
            clipboard_data_missing as *const (),
            "snapshot unavailable",
        ),
        ("GlobalLock", no_handle as *const (), "snapshot"),
        ("GlobalAlloc", allocation_failed as *const (), "allocation"),
        (
            "EmptyClipboard",
            clipboard_clear_failed as *const (),
            "clear",
        ),
        ("SetClipboardData", clipboard_set_failed as *const (), "set"),
        ("GetClipboardOwner", no_owner as *const (), "changed"),
    ] {
        write_clipboard("Previous copy").unwrap();
        let hook = unsafe { os_hooks::Hook::new(symbol, replacement) };
        let error = match super::os::snapshot_and_write("Cannot complete") {
            Ok(_) => panic!("{symbol} failed silently"),
            Err(error) => error,
        };
        drop(hook);
        assert!(error.contains(message), "{symbol}: {error}");
        write_clipboard("Recovered").unwrap();
        assert_eq!(read(), "Recovered");
    }
    for (symbol, replacement, message) in [
        ("GlobalAlloc", allocation_failed as *const (), "allocation"),
        ("GlobalLock", no_handle as *const (), "lock"),
        (
            "EmptyClipboard",
            clipboard_clear_failed as *const (),
            "busy",
        ),
        (
            "SetClipboardData",
            clipboard_set_failed as *const (),
            "restore failed",
        ),
    ] {
        write_clipboard("Previous copy").unwrap();
        let generation = unsafe {
            os_hooks::Hook::new(
                "GetClipboardSequenceNumber",
                stable_clipboard_generation as *const (),
            )
        };
        let restore = super::os::snapshot_and_write("Temporary dictation").unwrap();
        let hook = unsafe { os_hooks::Hook::new(symbol, replacement) };
        let error = match restore() {
            Err(error) => error,
            Ok(()) => panic!("{symbol} restore failure was ignored"),
        };
        drop(hook);
        drop(generation);
        assert!(error.contains(message), "{symbol}: {error}");
        write_clipboard("Recovered").unwrap();
        assert_eq!(read(), "Recovered");
    }
}
