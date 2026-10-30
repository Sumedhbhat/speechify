//! Native objects with controlled OS permission, focus and input responses.
use super::{read, write_clipboard, CLIPBOARD};
use crate::{
    behavior_tests::wait_until,
    platform::{paste, Poller},
};
use libloading::Library;

#[test]
#[ignore = "Requires the native macOS fixture and disposable desktop supplied by test-coverage.sh"]
fn native_permission_focus_key_state_and_paste_outcomes_follow_the_desktop_contract() {
    assert_eq!(
        std::env::var("SPEECHIFY_TEST_DISPOSABLE_DESKTOP").as_deref(),
        Ok("1")
    );
    let _clipboard = CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner());
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.build/macos-desktop.dylib");
    if std::env::var("SPEECHIFY_TEST_MACOS_SHIM_ACTIVE").as_deref() != Ok("1") {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "platform::clipboard_behavior::desktop_behavior::native_permission_focus_key_state_and_paste_outcomes_follow_the_desktop_contract", "--include-ignored", "--nocapture"])
            .env("DYLD_INSERT_LIBRARIES", &path).env("SPEECHIFY_TEST_MACOS_SHIM_ACTIVE", "1")
            .output().unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    // The fixture is loaded in this test process. System utilities such as
    // pbcopy use arm64e and must not inherit our arm64 interposer.
    std::env::remove_var("DYLD_INSERT_LIBRARIES");
    unsafe {
        let fixture = Library::new(path).unwrap();
        let reset = fixture
            .get::<unsafe extern "C" fn(u32)>(b"fixture_reset\0")
            .unwrap();
        let key = fixture
            .get::<unsafe extern "C" fn(u16, bool)>(b"fixture_key\0")
            .unwrap();
        let events = fixture
            .get::<unsafe extern "C" fn() -> u32>(b"fixture_event_count\0")
            .unwrap();
        let event_key = fixture
            .get::<unsafe extern "C" fn(u32) -> u16>(b"fixture_event_key\0")
            .unwrap();
        let event_down = fixture
            .get::<unsafe extern "C" fn(u32) -> bool>(b"fixture_event_down\0")
            .unwrap();
        let event_flags = fixture
            .get::<unsafe extern "C" fn(u32) -> u64>(b"fixture_event_flags\0")
            .unwrap();
        for denied in [1, 2] {
            reset(denied);
            assert!(Poller::new().is_err());
        }
        reset(0);
        let poller = Poller::new().unwrap();
        assert!(poller.down().unwrap().is_empty());
        key(0, true);
        key(55, true);
        key(127, true);
        assert_eq!(
            poller.down().unwrap(),
            ["a".into(), "cmd_l".into(), "key-127".into()]
                .into_iter()
                .collect()
        );
        reset(0);
        let target = poller.focus().unwrap();
        assert_eq!(poller.focus().unwrap(), target);
        for unavailable in [4, 8, 16, 64] {
            reset(unavailable);
            assert!(poller.focus().is_err());
        }
        reset(0);
        write_clipboard("Earlier copy 🦀").unwrap();
        assert!(paste(target ^ 1, "wrong target").is_err());
        key(0, true);
        assert!(paste(target, "keys still held").is_err());
        assert_eq!(read(), "Earlier copy 🦀");
        reset(0);
        assert!(paste(target, "A new dictation").unwrap().is_none());
        assert_eq!(events(), 4);
        assert_eq!(
            (event_key(0), event_down(0), event_flags(0)),
            (55, true, 1 << 20)
        );
        assert_eq!(
            (event_key(1), event_down(1), event_flags(1)),
            (9, true, 1 << 20)
        );
        assert_eq!(
            (event_key(2), event_down(2), event_flags(2)),
            (9, false, 1 << 20)
        );
        assert_eq!(
            (event_key(3), event_down(3), event_flags(3)),
            (55, false, 0)
        );
        wait_until(|| read() == "Earlier copy 🦀");
        reset(32);
        assert!(paste(target, "Cannot create a paste event").is_err());
        assert_eq!(events(), 0);
        wait_until(|| read() == "Earlier copy 🦀");
    }
}
