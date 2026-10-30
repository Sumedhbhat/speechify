use super::*;
use crate::behavior_tests::Home;
use std::io::Write;

fn device(home: &Home, name: &str) -> File {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let path = home.0.join(name);
    let cpath = CString::new(path.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) }, 0);
    OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .unwrap()
}
fn event(file: &mut File, kind: u16, code: u16, value: i32) {
    let mut event: libc::input_event = unsafe { std::mem::zeroed() };
    event.type_ = kind;
    event.code = code;
    event.value = value;
    let bytes = unsafe {
        std::slice::from_raw_parts(
            (&event as *const libc::input_event).cast::<u8>(),
            std::mem::size_of_val(&event),
        )
    };
    file.write_all(bytes).unwrap();
}

#[test]
fn keyboard_stream_tracks_press_repeat_release_and_multiple_devices() {
    let home = Home::new();
    let mut one = device(&home, "one-event-kbd");
    let mut two = device(&home, "two-event-kbd");
    // Non-keyboard devices are never opened or read.
    std::fs::write(home.0.join("mouse-event-mouse"), b"invalid events").unwrap();
    let keyboard = Keyboard::open_at(&home.0).unwrap();
    assert!(keyboard.down().unwrap().is_empty());
    event(&mut one, 1, 97, 1);
    event(&mut two, 1, 46, 1);
    event(&mut one, 1, 97, 2);
    event(&mut one, 0, 0, 0);
    assert_eq!(
        keyboard.down().unwrap(),
        ["ctrl_r".into(), "c".into()].into_iter().collect()
    );
    event(&mut two, 1, 46, 0);
    assert_eq!(
        keyboard.down().unwrap(),
        ["ctrl_r".into()].into_iter().collect()
    );
    event(&mut one, 1, 97, 0);
    assert!(keyboard.down().unwrap().is_empty());
    drop(one);
    assert!(keyboard.down().unwrap_err().contains("disconnected"));
}

#[test]
fn overflow_truncated_and_unreadable_events_cancel_instead_of_leaving_a_held_key() {
    let home = Home::new();
    let mut writer = device(&home, "one-event-kbd");
    let keyboard = Keyboard::open_at(&home.0).unwrap();
    event(&mut writer, 1, 97, 1);
    event(&mut writer, 0, 3, 0);
    assert!(keyboard.down().unwrap_err().contains("overflowed"));
    assert!(keyboard.down().unwrap().is_empty());
    writer.write_all(&[1, 2, 3]).unwrap();
    assert!(keyboard.down().unwrap_err().contains("Incomplete"));
    let directory = Home::new();
    std::fs::create_dir(directory.0.join("directory-event-kbd")).unwrap();
    let keyboard = Keyboard::open_at(&directory.0).unwrap();
    assert!(keyboard.down().is_err());
}

#[test]
fn missing_keyboard_access_reports_the_required_permission_without_changing_it() {
    let home = Home::new();
    assert!(Keyboard::open_at(&home.0.join("absent"))
        .err()
        .unwrap()
        .contains("explicit read permission"));
    assert!(Keyboard::open_at(&home.0)
        .err()
        .unwrap()
        .contains("No readable keyboard"));
    #[cfg(unix)]
    std::os::unix::fs::symlink(home.0.join("absent"), home.0.join("broken-event-kbd")).unwrap();
    assert!(Keyboard::open_at(&home.0).is_err());
}

#[test]
fn physical_key_codes_match_the_shortcut_names_exposed_in_settings() {
    for (code, name) in [
        (1, "esc"),
        (14, "backspace"),
        (15, "tab"),
        (28, "enter"),
        (29, "ctrl_l"),
        (42, "shift_l"),
        (54, "shift_r"),
        (56, "alt_l"),
        (57, "space"),
        (58, "caps_lock"),
        (70, "scroll_lock"),
        (97, "ctrl_r"),
        (100, "alt_r"),
        (103, "up"),
        (105, "left"),
        (106, "right"),
        (108, "down"),
        (125, "cmd_l"),
        (126, "cmd_r"),
        (2, "1"),
        (16, "q"),
        (30, "a"),
        (44, "z"),
        (59, "f1"),
        (68, "f10"),
        (87, "f11"),
        (88, "f12"),
        (183, "f13"),
        (194, "f24"),
        (250, "key-250"),
    ] {
        assert_eq!(key_name(code), name);
    }
}
