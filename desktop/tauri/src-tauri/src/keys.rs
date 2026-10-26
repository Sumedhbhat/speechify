use crate::config::Result;
use std::collections::BTreeSet;
pub fn parse(s: &str) -> Result<String> {
    let mut p: Vec<_> = s
        .split('+')
        .map(|x| x.trim().trim_matches(['<', '>']).to_lowercase())
        .collect();
    let last = p.pop().ok_or("Empty shortcut")?;
    if last.is_empty()
        || p.iter()
            .any(|x| !["ctrl", "alt", "shift", "cmd"].contains(&x.as_str()))
    {
        return Err("Use a key or a combination such as <ctrl>+<alt>+f".into());
    }
    let valid = last.len() == 1 && last.chars().all(|c| c.is_ascii_alphanumeric())
        || [
            "ctrl_l",
            "ctrl_r",
            "alt_l",
            "alt_r",
            "shift_l",
            "shift_r",
            "cmd_l",
            "cmd_r",
            "esc",
            "space",
            "enter",
            "backspace",
            "tab",
            "left",
            "right",
            "up",
            "down",
            "caps_lock",
            "scroll_lock",
        ]
        .contains(&last.as_str())
        || last
            .strip_prefix('f')
            .and_then(|n| n.parse::<u8>().ok())
            .is_some_and(|n| (1..=24).contains(&n));
    if !valid {
        return Err(format!("Unsupported shortcut key: {last}"));
    }
    p.sort();
    p.dedup();
    p.push(last);
    Ok(p.join("+"))
}
fn modifier(k: &str) -> &str {
    match k {
        "ctrl_l" | "ctrl_r" => "ctrl",
        "alt_l" | "alt_r" => "alt",
        "shift_l" | "shift_r" => "shift",
        "cmd_l" | "cmd_r" => "cmd",
        _ => k,
    }
}
#[derive(Debug, PartialEq)]
pub enum Action {
    Start,
    Cancel,
    Submit,
}
pub struct Hold {
    pub down: BTreeSet<String>,
    start: Option<u64>,
    cancelled: bool,
    pub combo: String,
    pub minimum: u64,
}
impl Hold {
    pub fn new(combo: &str, minimum: u64) -> Self {
        Self {
            down: BTreeSet::new(),
            start: None,
            cancelled: false,
            combo: parse(combo).unwrap_or_default(),
            minimum,
        }
    }
    pub fn event(&mut self, key: &str, pressed: bool, ms: u64) -> Option<Action> {
        if pressed {
            if !self.down.insert(key.into()) {
                return None;
            }
            let parts: Vec<_> = self.combo.split('+').collect();
            let last = parts.last().copied().unwrap_or("");
            let mods: BTreeSet<_> = self
                .down
                .iter()
                .filter(|k| k.as_str() != key)
                .map(|k| modifier(k))
                .collect();
            let want: BTreeSet<_> = parts[..parts.len().saturating_sub(1)]
                .iter()
                .copied()
                .collect();
            if key == last && mods == want && self.start.is_none() {
                self.start = Some(ms);
                self.cancelled = false;
                return Some(Action::Start);
            }
            if self.start.is_some() {
                self.cancelled = true;
                return Some(Action::Cancel);
            }
            None
        } else {
            self.down.remove(key);
            let active = self.start?;
            if self
                .combo
                .split('+')
                .any(|k| k == key || k == modifier(key))
            {
                self.start = None;
                if self.cancelled || ms.saturating_sub(active) < self.minimum {
                    Some(Action::Cancel)
                } else {
                    Some(Action::Submit)
                }
            } else {
                None
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn taps_repeats_and_ctrl_c() {
        let mut h = Hold::new("ctrl_r", 250);
        assert_eq!(h.event("ctrl_r", true, 0), Some(Action::Start));
        assert_eq!(h.event("ctrl_r", true, 100), None);
        assert_eq!(h.event("ctrl_r", false, 200), Some(Action::Cancel));
        h.event("ctrl_r", true, 1000);
        assert_eq!(h.event("c", true, 1300), Some(Action::Cancel));
        assert_eq!(h.event("ctrl_r", false, 1500), Some(Action::Cancel));
    }
    #[test]
    fn combination() {
        let mut h = Hold::new("<ctrl>+<alt>+f", 250);
        h.event("ctrl_l", true, 0);
        h.event("alt_l", true, 1);
        assert_eq!(h.event("f", true, 2), Some(Action::Start));
        assert_eq!(h.event("f", false, 500), Some(Action::Submit));
    }
}
