//! Shared safe-paste policy with a native adapter selected at compile time.
use crate::config::Result;

#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod os;
#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod os;
#[cfg(windows)]
#[path = "windows.rs"]
mod os;

#[cfg(all(test, target_os = "linux"))]
#[path = "../../tests/unit/platform.rs"]
mod behavior;
#[cfg(all(test, not(target_os = "linux")))]
#[path = "../../tests/unit/clipboard.rs"]
mod clipboard_behavior;

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
