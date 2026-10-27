//! Direct replacement is limited to standard Windows Edit controls. Browser,
//! terminal, rich editor, X11 and Wayland targets have no proven text range here.
use crate::config::Result;
#[derive(Debug)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct Receipt {
    pub target: u64,
    pub expected: Vec<u16>,
    pub start: usize,
    pub end: usize,
    pub process: u32,
    pub created: std::time::Instant,
}
#[cfg(any(windows, test))]
pub fn expected(before: &[u16], start: usize, end: usize, text: &str) -> Option<(Vec<u16>, usize)> {
    if start > end || end > before.len() {
        return None;
    }
    let insert: Vec<u16> = text.encode_utf16().collect();
    let caret = start + insert.len();
    let mut value = before[..start].to_vec();
    value.extend(insert);
    value.extend_from_slice(&before[end..]);
    Some((value, caret))
}
#[cfg(not(windows))]
pub fn capture(_target: u64, _text: &str) -> Option<Receipt> {
    None
}
#[cfg(not(windows))]
pub fn replace(_receipt: &Receipt, _text: &str) -> Result<()> {
    Err("Saved correction only: this platform cannot prove the original text range and caret. Copy the correction manually.".into())
}
#[cfg(windows)]
mod windows {
    use super::*;
    use std::ffi::c_void;
    #[link(name = "user32")]
    extern "system" {
        fn GetClassNameW(window: *mut c_void, name: *mut u16, max: i32) -> i32;
        fn GetWindowThreadProcessId(window: *mut c_void, pid: *mut u32) -> u32;
        fn SendMessageTimeoutW(
            window: *mut c_void,
            msg: u32,
            w: usize,
            l: isize,
            flags: u32,
            timeout: u32,
            result: *mut usize,
        ) -> isize;
    }
    fn send(target: u64, msg: u32, w: usize, l: isize) -> Result<usize> {
        unsafe {
            let mut result = 0;
            if SendMessageTimeoutW(target as _, msg, w, l, 0x0002 | 0x0020, 250, &mut result) == 0 {
                return Err("Target is unavailable, busy or elevated; replacement refused".into());
            }
            Ok(result)
        }
    }
    fn state(target: u64) -> Result<(u32, Vec<u16>, usize, usize)> {
        unsafe {
            let mut name = [0u16; 64];
            let n = GetClassNameW(target as _, name.as_mut_ptr(), 64);
            if n <= 0 || String::from_utf16_lossy(&name[..n as usize]) != "Edit" {
                return Err("Replacement requires a standard Windows text edit control".into());
            }
            let mut process = 0;
            if GetWindowThreadProcessId(target as _, &mut process) == 0 {
                return Err("Target closed".into());
            }
            // Use output buffers for full 32-bit selection offsets (not packed 16-bit return).
            let (mut start, mut end) = (0u32, 0u32);
            send(
                target,
                0x00b0,
                &mut start as *mut _ as usize,
                &mut end as *mut _ as isize,
            )?;
            let len = send(target, 0x000e, 0, 0)?;
            if len > 1_000_000 {
                return Err("Target is too large for guarded replacement".into());
            }
            let mut text = vec![0u16; len + 1];
            let n = send(target, 0x000d, text.len(), text.as_mut_ptr() as isize)?;
            text.truncate(n);
            Ok((process, text, start as usize, end as usize))
        }
    }
    pub fn capture(target: u64, text: &str) -> Option<Receipt> {
        let (process, before, start, end) = state(target).ok()?;
        let (expected, end) = super::expected(&before, start, end, text)?;
        Some(Receipt {
            target,
            expected,
            start,
            end,
            process,
            created: std::time::Instant::now(),
        })
    }
    pub fn replace(receipt: &Receipt, text: &str) -> Result<()> {
        if receipt.created.elapsed() > std::time::Duration::from_secs(120) {
            return Err("Replacement expired after two minutes; correction saved only".into());
        }
        let (process, current, start, end) = state(receipt.target)?;
        if process != receipt.process
            || current != receipt.expected
            || start != receipt.end
            || end != receipt.end
        {
            return Err("Original text or caret changed; correction saved only".into());
        }
        send(receipt.target, 0x00b1, receipt.start, receipt.end as isize)?; // EM_SETSEL
        let (process, current, start, end) = state(receipt.target)?;
        if process != receipt.process
            || current != receipt.expected
            || start != receipt.start
            || end != receipt.end
        {
            return Err("Target changed during replacement guard; correction saved only".into());
        }
        let text: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        send(receipt.target, 0x00c2, 1, text.as_ptr() as isize)?; // EM_REPLACESEL, undoable
        Ok(())
    }
}
#[cfg(windows)]
pub use windows::{capture, replace};
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacement_range_is_utf16_and_preserves_surroundings() {
        let before: Vec<u16> = "A old Z".encode_utf16().collect();
        let (after, caret) = expected(&before, 2, 5, "🦀\nnew").unwrap();
        assert_eq!(String::from_utf16(&after).unwrap(), "A 🦀\nnew Z");
        assert_eq!(caret, 8);
        assert!(expected(&before, 5, 2, "x").is_none());
        assert!(expected(&before, 0, 999, "x").is_none());
    }
}
