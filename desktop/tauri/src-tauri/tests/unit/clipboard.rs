//! Native clipboard tests run on disposable macOS/Windows CI desktops.
use super::*;
static CLIPBOARD: std::sync::Mutex<()> = std::sync::Mutex::new(());
#[cfg(windows)]
#[path = "windows.rs"]
mod desktop_behavior;
#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod desktop_behavior;

#[cfg(target_os = "macos")]
fn read() -> String {
    let output = Command::new("/usr/bin/pbpaste").output().unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap()
}
#[cfg(windows)]
fn read() -> String {
    use std::ffi::c_void;
    #[link(name = "user32")]
    extern "system" {
        fn OpenClipboard(owner: *mut c_void) -> i32;
        fn CloseClipboard() -> i32;
        fn GetClipboardData(format: u32) -> *mut c_void;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalLock(handle: *mut c_void) -> *mut c_void;
        fn GlobalUnlock(handle: *mut c_void) -> i32;
        fn GlobalSize(handle: *mut c_void) -> usize;
    }
    struct Clipboard;
    impl Drop for Clipboard {
        fn drop(&mut self) {
            unsafe {
                CloseClipboard();
            }
        }
    }
    unsafe {
        crate::behavior_tests::wait_until(|| OpenClipboard(std::ptr::null_mut()) != 0);
        let _clipboard = Clipboard;
        let handle = GetClipboardData(13);
        assert!(!handle.is_null());
        let data = GlobalLock(handle).cast::<u16>();
        assert!(!data.is_null());
        let buffer = std::slice::from_raw_parts(data, GlobalSize(handle) / 2);
        let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
        let text = String::from_utf16(&buffer[..end]).unwrap();
        GlobalUnlock(handle);
        text
    }
}

#[test]
#[ignore = "Mutates the OS clipboard; run on a disposable desktop with --include-ignored"]
fn native_clipboard_preserves_unicode_restores_previous_copy_and_protects_newer_copies() {
    let _clipboard = CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner());
    write_clipboard("Earlier clipboard café 🦀\nsecond line").unwrap();
    let restore = os::snapshot_and_write("A new dictation").unwrap();
    assert_eq!(read(), "A new dictation");
    restore().unwrap();
    assert_eq!(read(), "Earlier clipboard café 🦀\nsecond line");
    let restore = os::snapshot_and_write("Identical text").unwrap();
    write_clipboard("Identical text").unwrap();
    restore().unwrap();
    assert_eq!(read(), "Identical text");
    let restore = os::snapshot_and_write("Temporary").unwrap();
    write_clipboard("A different new copy").unwrap();
    restore().unwrap();
    assert_eq!(read(), "A different new copy");
    write_clipboard("").unwrap();
    let restore = os::snapshot_and_write("Temporary").unwrap();
    restore().unwrap();
    assert_eq!(read(), "");
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "Mutates the OS clipboard; run on a disposable desktop with --include-ignored"]
fn macos_rich_clipboard_data_survives_a_transient_dictation() {
    let _clipboard = CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner());
    // Write an explicit native RTF type. pbcopy/pbpaste may coerce it to text.
    let swift = |script: &str| {
        let output = Command::new("/usr/bin/swift")
            .args(["-e", script])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    };
    let original = r"{\rtf1\ansi A bold {\b word}}";
    swift(
        r##"import AppKit
let pb = NSPasteboard.general
pb.clearContents()
guard pb.setData(Data(#"{\rtf1\ansi A bold {\b word}}"#.utf8), forType: .rtf) else { exit(1) }
"##,
    );
    let read_rtf = r#"import AppKit
guard let data = NSPasteboard.general.data(forType: .rtf) else { exit(1) }
FileHandle.standardOutput.write(data)
"#;
    assert_eq!(swift(read_rtf), original.as_bytes());
    let restore = os::snapshot_and_write("Dictation").unwrap();
    assert_eq!(read(), "Dictation");
    restore().unwrap();
    assert_eq!(swift(read_rtf), original.as_bytes());
}

#[cfg(windows)]
#[test]
#[ignore = "Mutates the OS clipboard; run on a disposable desktop with --include-ignored"]
fn windows_bitmap_clipboard_is_refused_without_losing_the_owned_image() {
    use std::ffi::c_void;
    let _clipboard = CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner());
    #[link(name = "user32")]
    extern "system" {
        fn CreateWindowExW(
            ex: u32,
            class: *const u16,
            title: *const u16,
            style: u32,
            x: i32,
            y: i32,
            w: i32,
            h: i32,
            parent: *mut c_void,
            menu: *mut c_void,
            instance: *mut c_void,
            param: *mut c_void,
        ) -> *mut c_void;
        fn DestroyWindow(window: *mut c_void) -> i32;
        fn OpenClipboard(owner: *mut c_void) -> i32;
        fn CloseClipboard() -> i32;
        fn EmptyClipboard() -> i32;
        fn SetClipboardData(format: u32, data: *mut c_void) -> *mut c_void;
        fn GetClipboardData(format: u32) -> *mut c_void;
    }
    #[link(name = "gdi32")]
    extern "system" {
        fn CreateBitmap(w: i32, h: i32, planes: u32, bits: u32, data: *const c_void)
            -> *mut c_void;
    }
    struct Window(*mut c_void);
    impl Drop for Window {
        fn drop(&mut self) {
            unsafe {
                DestroyWindow(self.0);
            }
        }
    }
    unsafe {
        let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
        let null = std::ptr::null_mut();
        let window = Window(CreateWindowExW(
            0,
            class.as_ptr(),
            std::ptr::null(),
            0,
            0,
            0,
            1,
            1,
            null,
            null,
            null,
            null,
        ));
        assert!(!window.0.is_null());
        assert_ne!(OpenClipboard(window.0), 0);
        assert_ne!(EmptyClipboard(), 0);
        let bitmap = CreateBitmap(1, 1, 1, 1, [0u8; 2].as_ptr().cast());
        assert!(!bitmap.is_null());
        assert_eq!(SetClipboardData(2, bitmap), bitmap);
        CloseClipboard();
        let refused = os::snapshot_and_write("Must not overwrite the bitmap")
            .err()
            .unwrap();
        assert!(refused.contains("OS handle format"));
        assert_ne!(OpenClipboard(window.0), 0);
        let preserved = GetClipboardData(2);
        CloseClipboard();
        assert_eq!(preserved, bitmap);
    }
    write_clipboard("Test finished").unwrap();
}
