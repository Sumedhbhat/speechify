//! Shortcut parsing and chord tracking over raw key state.
//!
//! The shell reports which keys are down; this decides which shortcuts
//! went down or up. Working from state rather than key events means a
//! missed key-up can never leave dictation stuck on.

use crate::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Key {
    Ctrl,
    Alt,
    Shift,
    /// Windows key, Command on a Mac, Super on Linux.
    Meta,
    Space,
    Escape,
    CapsLock,
    Enter,
    Tab,
    /// A letter or digit, uppercase.
    Char(char),
    /// F1 to F24.
    Function(u8),
    /// Any key that cannot be part of a shortcut. It still counts as held,
    /// so Ctrl+Meta+Left is not mistaken for Ctrl+Meta.
    Other,
}

impl Key {
    pub fn is_modifier(self) -> bool {
        matches!(self, Key::Ctrl | Key::Alt | Key::Shift | Key::Meta)
    }

    /// Parses a key name as reported by the operating-system layer;
    /// unknown names are `Other`.
    pub fn from_name(name: &str) -> Key {
        Key::parse(name).unwrap_or(Key::Other)
    }

    fn parse(name: &str) -> Result<Key> {
        let lower = name.trim().to_lowercase();
        Ok(match lower.as_str() {
            "ctrl" | "control" => Key::Ctrl,
            "alt" | "option" | "opt" => Key::Alt,
            "shift" => Key::Shift,
            "meta" | "win" | "windows" | "cmd" | "command" | "super" => Key::Meta,
            "space" => Key::Space,
            "escape" | "esc" => Key::Escape,
            "capslock" | "caps lock" | "caps" => Key::CapsLock,
            "enter" | "return" => Key::Enter,
            "tab" => Key::Tab,
            _ => {
                let mut chars = lower.chars();
                match (chars.next(), chars.as_str()) {
                    (Some(c), "") if c.is_ascii_alphanumeric() => Key::Char(c.to_ascii_uppercase()),
                    (Some('f'), number) => match number.parse::<u8>() {
                        Ok(n) if (1..=24).contains(&n) => Key::Function(n),
                        _ => return Err(format!("Unknown key \"{}\"", name.trim())),
                    },
                    _ => return Err(format!("Unknown key \"{}\"", name.trim())),
                }
            }
        })
    }

    fn name(self) -> String {
        match self {
            Key::Char(c) => c.to_string(),
            Key::Function(n) => format!("F{n}"),
            other => format!("{other:?}"),
        }
    }
}

/// Keys held together, such as Ctrl+Meta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chord(BTreeSet<Key>);

impl Chord {
    /// Parses "Ctrl+Meta+Space". A shortcut has one to three keys and
    /// needs a modifier, except Escape, Caps Lock and function keys,
    /// which are safe to bind alone.
    pub fn parse(text: &str) -> Result<Chord> {
        let keys = text.split('+').map(Key::parse).collect::<Result<BTreeSet<Key>>>()?;
        let alone_ok = |key: &Key| matches!(key, Key::Escape | Key::CapsLock | Key::Function(_));
        if keys.len() > 3 {
            return Err("A shortcut can have at most three keys".into());
        }
        if !keys.iter().any(|key| key.is_modifier() || alone_ok(key)) {
            return Err("A shortcut needs Ctrl, Alt, Shift or the Windows/Command key".into());
        }
        Ok(Chord(keys))
    }

    pub fn keys(&self) -> &BTreeSet<Key> {
        &self.0
    }
}

impl std::fmt::Display for Chord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<String> = self.0.iter().map(|key| key.name()).collect();
        write!(f, "{}", names.join("+"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Bind {
    PushToTalk,
    HandsFree,
    Command,
    PasteLast,
    CopyLast,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Down(Bind),
    Up(Bind),
    /// Every key is up again; deferred pastes are safe now.
    Released,
}

pub struct Tracker {
    bindings: Vec<(Bind, Chord, bool)>,
    any_down: bool,
}

impl Tracker {
    pub fn new(hotkeys: &crate::settings::Hotkeys) -> Result<Tracker> {
        let pairs = [
            (Bind::PushToTalk, &hotkeys.push_to_talk),
            (Bind::HandsFree, &hotkeys.hands_free),
            (Bind::Command, &hotkeys.command),
            (Bind::PasteLast, &hotkeys.paste_last),
            (Bind::CopyLast, &hotkeys.copy_last),
            (Bind::Cancel, &hotkeys.cancel),
        ];
        let mut bindings = Vec::new();
        for (bind, text) in pairs {
            // An empty shortcut switches the action off.
            if !text.trim().is_empty() {
                bindings.push((bind, Chord::parse(text)?, false));
            }
        }
        Ok(Tracker {
            bindings,
            any_down: false,
        })
    }

    /// Feeds the current key state and returns what changed.
    pub fn update(&mut self, pressed: &BTreeSet<Key>) -> Vec<Edge> {
        let mut edges = Vec::new();
        let chords: Vec<Chord> = self.bindings.iter().map(|(_, chord, _)| chord.clone()).collect();
        for (bind, chord, active) in &mut self.bindings {
            let held = chord.0.is_subset(pressed);
            if *active && !held {
                *active = false;
                edges.push(Edge::Up(*bind));
            } else if !*active && held {
                // Extra keys are fine only when they extend this shortcut into
                // another one (Ctrl+Meta → Ctrl+Meta+Space). Ctrl+Meta+Left is
                // somebody else's shortcut and must not start dictation.
                let foreign = !chords
                    .iter()
                    .any(|other| other.0.is_superset(&chord.0) && pressed.is_subset(&other.0));
                if !foreign {
                    *active = true;
                    edges.push(Edge::Down(*bind));
                }
            }
        }
        if self.any_down && pressed.is_empty() {
            edges.push(Edge::Released);
        }
        self.any_down = !pressed.is_empty();
        edges
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Hotkeys;

    fn keys(list: &[Key]) -> BTreeSet<Key> {
        list.iter().copied().collect()
    }

    fn windows() -> Hotkeys {
        Hotkeys {
            push_to_talk: "Ctrl+Win".into(),
            hands_free: "Ctrl+Win+Space".into(),
            command: "Ctrl+Win+Alt".into(),
            paste_last: "Alt+Shift+Z".into(),
            copy_last: String::new(),
            cancel: "Esc".into(),
        }
    }

    #[test]
    fn parses_names_and_aliases() {
        assert_eq!(
            Chord::parse("control + cmd + space").unwrap().to_string(),
            "Ctrl+Meta+Space"
        );
        assert_eq!(Chord::parse("Alt+Shift+z").unwrap().to_string(), "Alt+Shift+Z");
        assert_eq!(Chord::parse("F13").unwrap().to_string(), "F13");
        assert_eq!(Chord::parse("option+return").unwrap().to_string(), "Alt+Enter");
        assert_eq!(Chord::parse("caps lock").unwrap().keys().len(), 1);
        assert_eq!(Chord::parse("Shift+Tab").unwrap().to_string(), "Shift+Tab");
        assert_eq!(Key::from_name("Meta"), Key::Meta);
        assert_eq!(Key::from_name("Other"), Key::Other);
    }

    #[test]
    fn rejects_unusable_shortcuts() {
        assert_eq!(
            Chord::parse("Z").unwrap_err(),
            "A shortcut needs Ctrl, Alt, Shift or the Windows/Command key"
        );
        assert_eq!(
            Chord::parse("Ctrl+Alt+Shift+Z").unwrap_err(),
            "A shortcut can have at most three keys"
        );
        assert_eq!(Chord::parse("Ctrl+Hyper").unwrap_err(), "Unknown key \"Hyper\"");
        assert_eq!(Chord::parse("F99").unwrap_err(), "Unknown key \"F99\"");
        assert_eq!(Chord::parse("").unwrap_err(), "Unknown key \"\"");
        assert!(Tracker::new(&Hotkeys {
            cancel: "Q".into(),
            ..windows()
        })
        .is_err());
    }

    #[test]
    fn hold_and_release() {
        let mut tracker = Tracker::new(&windows()).unwrap();
        assert_eq!(tracker.update(&keys(&[Key::Ctrl])), vec![]);
        assert_eq!(
            tracker.update(&keys(&[Key::Ctrl, Key::Meta])),
            vec![Edge::Down(Bind::PushToTalk)]
        );
        assert_eq!(tracker.update(&keys(&[Key::Ctrl, Key::Meta])), vec![]);
        assert_eq!(tracker.update(&keys(&[Key::Meta])), vec![Edge::Up(Bind::PushToTalk)]);
        assert_eq!(tracker.update(&keys(&[])), vec![Edge::Released]);
        assert_eq!(tracker.update(&keys(&[])), vec![]);
    }

    #[test]
    fn extending_a_shortcut_keeps_the_first_one_down() {
        let mut tracker = Tracker::new(&windows()).unwrap();
        tracker.update(&keys(&[Key::Ctrl, Key::Meta]));
        assert_eq!(
            tracker.update(&keys(&[Key::Ctrl, Key::Meta, Key::Space])),
            vec![Edge::Down(Bind::HandsFree)]
        );
        assert_eq!(
            tracker.update(&keys(&[])),
            vec![Edge::Up(Bind::PushToTalk), Edge::Up(Bind::HandsFree), Edge::Released]
        );
        // All three at once still starts both.
        assert_eq!(
            tracker.update(&keys(&[Key::Ctrl, Key::Meta, Key::Alt])),
            vec![Edge::Down(Bind::PushToTalk), Edge::Down(Bind::Command)]
        );
    }

    #[test]
    fn other_apps_shortcuts_do_not_trigger() {
        let mut tracker = Tracker::new(&windows()).unwrap();
        assert_eq!(tracker.update(&keys(&[Key::Ctrl, Key::Meta, Key::Char('D')])), vec![]);
        // Once it is a foreign combination, releasing the extra key starts normally.
        assert_eq!(
            tracker.update(&keys(&[Key::Ctrl, Key::Meta])),
            vec![Edge::Down(Bind::PushToTalk)]
        );
        // A key pressed while dictating does not interrupt it.
        assert_eq!(tracker.update(&keys(&[Key::Ctrl, Key::Meta, Key::Char('D')])), vec![]);
        assert_eq!(
            tracker.update(&keys(&[Key::Escape])),
            vec![Edge::Up(Bind::PushToTalk), Edge::Down(Bind::Cancel)]
        );
    }

    #[test]
    fn extra_keys_must_belong_to_a_single_extension() {
        let mut tracker = Tracker::new(&windows()).unwrap();
        assert!(tracker
            .update(&keys(&[Key::Ctrl, Key::Meta, Key::Space, Key::Alt]))
            .is_empty());
        assert_eq!(
            tracker.update(&keys(&[Key::Ctrl, Key::Meta, Key::Space])),
            vec![Edge::Down(Bind::PushToTalk), Edge::Down(Bind::HandsFree)]
        );
    }
}
