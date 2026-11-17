//! Typing text into other applications through the clipboard.

use arboard::Clipboard;
use enigo::{Direction, Enigo, Key, Keyboard as _, Settings};
use std::sync::Mutex;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteKeys {
    /// Ctrl+V, or Cmd+V on a Mac.
    Standard,
    /// Ctrl+Shift+V.
    WithShift,
    /// Shift+Insert.
    ShiftInsert,
}

// Keep the X11 owner alive. The mutex protects only clipboard operations,
// never sleeps or key injection. Generations invalidate older restore jobs.
struct Pending {
    generation: u64,
    text: String,
    previous: Option<String>,
    stamp: Option<u32>,
}

#[derive(Default)]
struct ClipboardState {
    clipboard: Option<Clipboard>,
    generation: u64,
    pending: Option<Pending>,
}

static CLIPBOARD: Mutex<ClipboardState> = Mutex::new(ClipboardState {
    clipboard: None,
    generation: 0,
    pending: None,
});

fn with_clipboard<T>(action: impl FnOnce(&mut ClipboardState) -> Result<T, arboard::Error>) -> Result<T, String> {
    let mut state = CLIPBOARD
        .lock()
        .map_err(|_| "The clipboard is unavailable.".to_string())?;
    if state.clipboard.is_none() {
        state.clipboard = Some(Clipboard::new().map_err(|e| format!("The clipboard is unavailable: {e}"))?);
    }
    action(&mut state).map_err(|e| format!("The clipboard is busy: {e}"))
}

impl ClipboardState {
    fn clipboard(&mut self) -> &mut Clipboard {
        self.clipboard.as_mut().expect("initialized by with_clipboard")
    }

    fn text(&mut self) -> Result<Option<String>, arboard::Error> {
        match self.clipboard().get_text() {
            Ok(text) => Ok(Some(text)),
            Err(arboard::Error::ContentNotAvailable) => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn begin(&mut self, text: &str) -> Result<u64, arboard::Error> {
        let current = self.text()?;
        let previous = previous_text(self.pending.as_ref(), self.generation, current, clipboard_stamp());
        set_transient(self.clipboard(), text)?;
        self.generation = self.generation.wrapping_add(1);
        self.pending = Some(Pending {
            generation: self.generation,
            text: text.into(),
            previous,
            stamp: clipboard_stamp(),
        });
        Ok(self.generation)
    }

    fn restore(&mut self, generation: u64) -> Result<(), arboard::Error> {
        if self
            .pending
            .as_ref()
            .is_none_or(|pending| pending.generation != generation)
        {
            return Ok(());
        }
        let current = self.text()?;
        let pending = self.pending.take().expect("checked above");
        if restore_matches(&pending, self.generation, current.as_deref(), clipboard_stamp()) {
            // A text-only adapter cannot reconstruct images or other formats.
            // Leave the dictated text rather than clearing unknown contents.
            if let Some(previous) = pending.previous {
                #[cfg(windows)]
                if let Some(stamp) = pending.stamp {
                    // arboard 3.7 opens and holds the Win32 clipboard in
                    // Set::new. Recheck under that OS lock so another app
                    // cannot copy between this check and the restore write.
                    let writer = self.clipboard().set();
                    if clipboard_stamp() == Some(stamp) {
                        writer.text(previous)?;
                    }
                }
                #[cfg(not(windows))]
                self.clipboard().set_text(previous)?;
            }
        }
        Ok(())
    }
}

fn previous_text(
    pending: Option<&Pending>,
    generation: u64,
    current: Option<String>,
    stamp: Option<u32>,
) -> Option<String> {
    match pending {
        Some(pending) if restore_matches(pending, generation, current.as_deref(), stamp) => pending.previous.clone(),
        _ => current,
    }
}

fn restore_matches(pending: &Pending, generation: u64, current: Option<&str>, stamp: Option<u32>) -> bool {
    pending.generation == generation && current == Some(pending.text.as_str()) && pending.stamp == stamp
}

#[cfg(windows)]
fn clipboard_stamp() -> Option<u32> {
    #[link(name = "user32")]
    extern "system" {
        fn GetClipboardSequenceNumber() -> u32;
    }
    let sequence = unsafe { GetClipboardSequenceNumber() };
    (sequence != 0).then_some(sequence)
}

#[cfg(not(windows))]
fn clipboard_stamp() -> Option<u32> {
    None
}

/// Puts dictated text on the clipboard, asking clipboard managers and
/// clipboard history not to record it.
fn set_transient(clipboard: &mut Clipboard, text: &str) -> Result<(), arboard::Error> {
    #[cfg(windows)]
    {
        use arboard::SetExtWindows;
        clipboard
            .set()
            .exclude_from_cloud()
            .exclude_from_history()
            .text(text.to_string())
    }
    #[cfg(target_os = "macos")]
    {
        use arboard::SetExtApple;
        clipboard.set().exclude_from_history().text(text.to_string())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        use arboard::SetExtLinux;
        clipboard.set().exclude_from_history().text(text.to_string())
    }
}

pub fn copy(text: &str) -> Result<(), String> {
    with_clipboard(|state| {
        state.clipboard().set_text(text.to_string())?;
        state.generation = state.generation.wrapping_add(1);
        state.pending = None;
        Ok(())
    })
}

fn modifier() -> Key {
    if cfg!(target_os = "macos") {
        Key::Meta
    } else {
        Key::Control
    }
}

fn chord(keys: &[Key], last: Key) -> Result<(), String> {
    let failed = |e: enigo::InputError| format!("Could not send keys to the app: {e}");
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| format!("Could not send keys to the app: {e}"))?;
    let mut pressed = Vec::new();
    let result = (|| {
        for key in keys {
            enigo.key(*key, Direction::Press).map_err(failed)?;
            pressed.push(*key);
        }
        enigo.key(last, Direction::Click).map_err(failed)
    })();
    for key in pressed.iter().rev() {
        let _ = enigo.key(*key, Direction::Release);
    }
    result
}

/// A timed-out wait must not inject a chord with the user's modifiers held.
fn wait_for_release() -> Result<(), String> {
    let keyboard = crate::keys::Keyboard::new();
    if !keyboard.available() {
        return Err("Speechify cannot read the keyboard. Check desktop and Accessibility permissions.".into());
    }
    for _ in 0..40 {
        if !keyboard
            .pressed()
            .iter()
            .any(|key| matches!(key.as_str(), "Ctrl" | "Alt" | "Shift" | "Meta"))
        {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    Err("Release the shortcut keys, then try pasting again.".into())
}

fn letter(value: char) -> Key {
    // Windows shortcut keys must be virtual keys even on layouts where the
    // Latin character is absent. Unicode input can become VK_PACKET text.
    #[cfg(windows)]
    {
        Key::Other(value.to_ascii_uppercase() as u32)
    }
    #[cfg(not(windows))]
    {
        Key::Unicode(value)
    }
}

/// Pastes `text` into the focused application, then puts the previous
/// clipboard text back after `restore_ms` unless the user has copied
/// something else in the meantime.
pub fn paste(text: &str, keys: PasteKeys, restore_ms: u64) -> Result<(), String> {
    if crate::is_wayland() {
        copy(text)?;
        return Err(
            "Speechify can't type into apps on Wayland yet. Your text is on the clipboard: paste it with Ctrl+V."
                .into(),
        );
    }
    wait_for_release()?;
    let generation = with_clipboard(|state| state.begin(text))?;
    std::thread::sleep(Duration::from_millis(40));
    let result = (|| {
        wait_for_release()?;
        let still_ours = with_clipboard(|state| {
            let current = state.text()?;
            Ok(state.pending.as_ref().is_some_and(|pending| {
                pending.generation == generation
                    && restore_matches(pending, state.generation, current.as_deref(), clipboard_stamp())
            }))
        })?;
        if !still_ours {
            return Err("The clipboard changed before pasting. Try pasting the dictation again.".into());
        }
        match keys {
            PasteKeys::Standard => chord(&[modifier()], letter('v')),
            PasteKeys::WithShift => chord(&[modifier(), Key::Shift], letter('v')),
            #[cfg(not(target_os = "macos"))]
            PasteKeys::ShiftInsert => chord(&[Key::Shift], Key::Insert),
            #[cfg(target_os = "macos")]
            PasteKeys::ShiftInsert => chord(&[modifier()], letter('v')),
        }
    })();
    // Restore even if injection fails. Never let an older job restore over a
    // newer paste or an explicit Copy command from one of our windows.
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(restore_ms));
        let _ = with_clipboard(|state| state.restore(generation));
    });
    result?;
    Ok(())
}

/// The text selected in the focused application, read by copying it.
/// The clipboard is restored afterwards. Empty when nothing is selected.
pub fn selection() -> String {
    if crate::is_wayland() {
        return String::new();
    }
    if wait_for_release().is_err() {
        return String::new();
    }
    // A marker distinguishes ignored Copy from a new selection. Use the same
    // backup and generation tracking as paste, including the failure path.
    let marker = format!(
        "Speechify selection {}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let Ok(generation) = with_clipboard(|state| state.begin(&marker)) else {
        return String::new();
    };
    if chord(&[modifier()], letter('c')).is_err() {
        let _ = with_clipboard(|state| state.restore(generation));
        return String::new();
    }
    let mut selected = String::new();
    for _ in 0..20 {
        std::thread::sleep(Duration::from_millis(25));
        let result = with_clipboard(|state| {
            if state.generation != generation {
                return Ok(Some(String::new()));
            }
            let current = state.text()?;
            if current.as_deref().is_some_and(|text| text != marker) {
                // The target's Copy changed the marker. Restore only while
                // those exact contents are still current.
                if let Some(pending) = &mut state.pending {
                    pending.text = current.clone().unwrap_or_default();
                    pending.stamp = clipboard_stamp();
                }
                return Ok(current);
            }
            Ok(None)
        });
        match result {
            Ok(Some(text)) => {
                selected = text;
                break;
            }
            Ok(None) => {}
            Err(_) => break,
        }
    }
    let _ = with_clipboard(|state| state.restore(generation));
    selected
}

pub fn press_enter() {
    std::thread::sleep(Duration::from_millis(80));
    if wait_for_release().is_ok() {
        let _ = chord(&[], Key::Return);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_letters_are_keys_instead_of_inserted_text() {
        #[cfg(windows)]
        assert_eq!(letter('v'), Key::Other(0x56));
        #[cfg(not(windows))]
        assert_eq!(letter('v'), Key::Unicode('v'));
    }

    #[test]
    fn restore_requires_our_generation_contents_and_sequence() {
        let pending = Pending {
            generation: 4,
            text: "dictation".into(),
            previous: Some("before".into()),
            stamp: Some(12),
        };
        assert!(restore_matches(&pending, 4, Some("dictation"), Some(12)));
        assert!(!restore_matches(&pending, 5, Some("dictation"), Some(12)));
        assert!(!restore_matches(&pending, 4, Some("new copy"), Some(12)));
        assert!(!restore_matches(&pending, 4, None, Some(12)));
        // Copying identical text still changes the Windows sequence number.
        assert!(!restore_matches(&pending, 4, Some("dictation"), Some(13)));
    }

    #[test]
    fn overlapping_pastes_restore_the_original_but_preserve_new_copies() {
        let pending = Pending {
            generation: 2,
            text: "first dictation".into(),
            previous: Some("original clipboard".into()),
            stamp: Some(10),
        };
        assert_eq!(
            previous_text(Some(&pending), 2, Some("first dictation".into()), Some(10)).as_deref(),
            Some("original clipboard")
        );
        assert_eq!(
            previous_text(Some(&pending), 2, Some("user copy".into()), Some(11)).as_deref(),
            Some("user copy")
        );
        assert_eq!(
            previous_text(Some(&pending), 2, Some("first dictation".into()), Some(11)).as_deref(),
            Some("first dictation")
        );
        assert_eq!(
            previous_text(None, 3, Some("explicit copy".into()), Some(12)).as_deref(),
            Some("explicit copy")
        );
    }

    #[test]
    fn text_only_platforms_still_check_contents() {
        let pending = Pending {
            generation: 1,
            text: "text".into(),
            previous: None,
            stamp: None,
        };
        assert!(restore_matches(&pending, 1, Some("text"), None));
        assert!(!restore_matches(&pending, 1, Some("different"), None));
    }
}
