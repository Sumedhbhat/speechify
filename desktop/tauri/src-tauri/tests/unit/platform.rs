use super::*;
use libloading::Library;
use std::ffi::{c_char, c_void};

struct Window {
    lib: Library,
    xtest: Library,
    display: *mut c_void,
    id: u64,
}
impl Window {
    unsafe fn function<T: Copy>(&self, name: &[u8]) -> T {
        *self.lib.get::<T>(name).unwrap()
    }
    fn new() -> Self {
        unsafe {
            let lib = Library::new("libX11.so.6").unwrap();
            let open = lib
                .get::<unsafe extern "C" fn(*const c_char) -> *mut c_void>(b"XOpenDisplay\0")
                .unwrap();
            let display = open(std::ptr::null());
            assert!(!display.is_null(), "tests require an isolated Xvfb display");
            let root = lib
                .get::<unsafe extern "C" fn(*mut c_void) -> u64>(b"XDefaultRootWindow\0")
                .unwrap();
            let create=lib.get::<unsafe extern "C" fn(*mut c_void,u64,i32,i32,u32,u32,u32,u64,u64)->u64>(b"XCreateSimpleWindow\0").unwrap();
            let id = create(display, root(display), 0, 0, 100, 100, 0, 0, 0);
            drop(create);
            drop(root);
            drop(open);
            let this = Self {
                lib,
                xtest: Library::new("libXtst.so.6").unwrap(),
                display,
                id,
            };
            let map: unsafe extern "C" fn(*mut c_void, u64) -> i32 = this.function(b"XMapWindow\0");
            map(display, id);
            let focus: unsafe extern "C" fn(*mut c_void, u64, i32, u64) -> i32 =
                this.function(b"XSetInputFocus\0");
            focus(display, id, 1, 0);
            this.sync();
            this
        }
    }
    fn sync(&self) {
        unsafe {
            let sync: unsafe extern "C" fn(*mut c_void, i32) -> i32 = self.function(b"XSync\0");
            sync(self.display, 0);
        }
    }
}
impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            let destroy: unsafe extern "C" fn(*mut c_void, u64) -> i32 =
                self.function(b"XDestroyWindow\0");
            destroy(self.display, self.id);
            let close: unsafe extern "C" fn(*mut c_void) -> i32 = self.function(b"XCloseDisplay\0");
            close(self.display);
        }
    }
}
fn key(action: &str, name: &str) {
    assert!(Command::new("xdotool")
        .args([action, name])
        .status()
        .unwrap()
        .success());
}

#[test]
fn x11_key_state_focus_guards_and_copy_match_the_native_contract() {
    let window = Window::new();
    let poller = Poller::new().unwrap();
    assert_eq!(poller.focus().unwrap(), window.id);
    assert!(poller.down().unwrap().is_empty());
    write_clipboard("Keep this copy 🦀").unwrap();
    let read = || {
        Command::new("xclip")
            .args(["-selection", "clipboard", "-o"])
            .output()
            .unwrap()
            .stdout
    };
    assert_eq!(String::from_utf8(read()).unwrap(), "Keep this copy 🦀");
    assert!(paste(window.id + 1, "wrong target")
        .unwrap_err()
        .contains("Focus changed"));
    key("keydown", "Control_R");
    window.sync();
    assert!(poller.down().unwrap().contains("ctrl_r"));
    let refused = paste(window.id, "must not paste");
    key("keyup", "Control_R");
    window.sync();
    assert!(refused.unwrap_err().contains("Keys are still held"));
    assert!(poller.down().unwrap().is_empty());
    assert_eq!(String::from_utf8(read()).unwrap(), "Keep this copy 🦀");
    key("keydown", "a");
    window.sync();
    assert!(poller.down().unwrap().contains("a"));
    key("keyup", "a");
    unsafe {
        let event = window
            .xtest
            .get::<unsafe extern "C" fn(*mut c_void, u32, i32, u64) -> i32>(b"XTestFakeKeyEvent\0")
            .unwrap();
        event(window.display, 8, 1, 0);
        window.sync();
        let held = poller.down().unwrap();
        event(window.display, 8, 0, 0);
        window.sync();
        assert!(held.contains("key-8"));
        let focus: unsafe extern "C" fn(*mut c_void, u64, i32, u64) -> i32 =
            window.function(b"XSetInputFocus\0");
        focus(window.display, 1, 1, 0);
        window.sync();
        assert!(poller.focus().is_err());
    }
}
