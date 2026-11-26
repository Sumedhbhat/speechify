//! Shortcut parsing and chord tracking over raw key state.
//!
//! The shell reports which keys are down; this decides which shortcuts
//! went down or up. Working from state rather than key events means a
//! missed key-up can never leave dictation stuck on.

use serde::{Deserialize, Serialize};
use speechify_core::Result;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Key {
    /// Either Ctrl key. The keyboard reports a side; a shortcut may name one.
    Ctrl,
    Alt,
    Shift,
    /// Windows key, Command on a Mac, Super on Linux.
    Meta,
    LeftCtrl,
    RightCtrl,
    LeftAlt,
    /// Also AltGr.
    RightAlt,
    LeftShift,
    RightShift,
    LeftMeta,
    RightMeta,
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
        self.either_side().is_some()
    }

    /// The modifier without its side: Right Shift is a Shift.
    fn either_side(self) -> Option<Key> {
        Some(match self {
            Key::Ctrl | Key::LeftCtrl | Key::RightCtrl => Key::Ctrl,
            Key::Alt | Key::LeftAlt | Key::RightAlt => Key::Alt,
            Key::Shift | Key::LeftShift | Key::RightShift => Key::Shift,
            Key::Meta | Key::LeftMeta | Key::RightMeta => Key::Meta,
            _ => return None,
        })
    }

    /// Whether holding `pressed` holds this shortcut key. Shift is held by
    /// either Shift key; Right Shift only by the right one.
    fn held_by(self, pressed: Key) -> bool {
        self == pressed || pressed.either_side() == Some(self)
    }

    /// Parses a key name as reported by the operating-system layer;
    /// unknown names are `Other`.
    pub fn from_name(name: &str) -> Key {
        Key::parse(name).unwrap_or(Key::Other)
    }

    fn parse(name: &str) -> Result<Key> {
        let lower: String = name
            .trim()
            .to_lowercase()
            .chars()
            .filter(|c| !matches!(c, ' ' | '_' | '-'))
            .collect();
        // "Right Shift", "Left Ctrl": one side of a modifier.
        for (prefix, left) in [("left", true), ("right", false)] {
            if let Some(rest) = lower.strip_prefix(prefix) {
                return match (Key::parse(rest), left) {
                    (Ok(Key::Ctrl), true) => Ok(Key::LeftCtrl),
                    (Ok(Key::Ctrl), false) => Ok(Key::RightCtrl),
                    (Ok(Key::Alt), true) => Ok(Key::LeftAlt),
                    (Ok(Key::Alt), false) => Ok(Key::RightAlt),
                    (Ok(Key::Shift), true) => Ok(Key::LeftShift),
                    (Ok(Key::Shift), false) => Ok(Key::RightShift),
                    (Ok(Key::Meta), true) => Ok(Key::LeftMeta),
                    (Ok(Key::Meta), false) => Ok(Key::RightMeta),
                    _ => Err(format!("Unknown key \"{}\"", name.trim())),
                };
            }
        }
        Ok(match lower.as_str() {
            "ctrl" | "control" => Key::Ctrl,
            "alt" | "option" | "opt" => Key::Alt,
            "shift" => Key::Shift,
            "meta" | "win" | "windows" | "cmd" | "command" | "super" => Key::Meta,
            "space" => Key::Space,
            "escape" | "esc" => Key::Escape,
            "capslock" | "caps" => Key::CapsLock,
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

    /// The name settings use; `parse` reads it back.
    fn name(self) -> String {
        match self {
            Key::Char(c) => c.to_string(),
            Key::Function(n) => format!("F{n}"),
            Key::LeftCtrl => "Left Ctrl".into(),
            Key::RightCtrl => "Right Ctrl".into(),
            Key::LeftAlt => "Left Alt".into(),
            Key::RightAlt => "Right Alt".into(),
            Key::LeftShift => "Left Shift".into(),
            Key::RightShift => "Right Shift".into(),
            Key::LeftMeta => "Left Meta".into(),
            Key::RightMeta => "Right Meta".into(),
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

    /// Whether these keys hold every key of the shortcut.
    fn held_by(&self, pressed: &BTreeSet<Key>) -> bool {
        self.0.iter().all(|key| pressed.iter().any(|&down| key.held_by(down)))
    }

    /// Whether holding this shortcut holds `other` too: Shift+Space extends
    /// Right Shift, since Right Shift+Space holds both.
    fn extends(&self, other: &Chord) -> bool {
        other
            .0
            .iter()
            .all(|&key| self.0.iter().any(|&mine| key.held_by(mine) || mine.held_by(key)))
    }

    /// Whether every key in `pressed` is part of the shortcut.
    fn covers(&self, pressed: &BTreeSet<Key>) -> bool {
        pressed.iter().all(|&down| self.0.iter().any(|key| key.held_by(down)))
    }

    /// Shift, Right Shift or Ctrl+Meta: keys people also hold to type
    /// capitals or other apps' shortcuts.
    fn modifiers_only(&self) -> bool {
        self.0.iter().all(|key| key.is_modifier())
    }
}

impl std::fmt::Display for Chord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<String> = self.0.iter().map(|key| key.name()).collect();
        write!(f, "{}", names.join("+"))
    }
}

const TWICE: &str = " x2";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcut {
    chord: Chord,
    double: bool,
}

impl Shortcut {
    pub fn parse(text: &str) -> Result<Shortcut> {
        let text = text.trim();
        let lower = text.to_ascii_lowercase();
        let (keys, double) = match lower.strip_suffix(TWICE.trim_start()) {
            Some(rest) if rest.ends_with(char::is_whitespace) => (&text[..rest.len()], true),
            _ => (text, false),
        };
        Ok(Shortcut {
            chord: Chord::parse(keys)?,
            double,
        })
    }
}

impl std::fmt::Display for Shortcut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", self.chord, if self.double { TWICE } else { "" })
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

impl Bind {
    const ALL: [Bind; 6] = [
        Bind::PushToTalk,
        Bind::HandsFree,
        Bind::Command,
        Bind::PasteLast,
        Bind::CopyLast,
        Bind::Cancel,
    ];

    fn shortcut(self, hotkeys: &speechify_core::settings::Hotkeys) -> &str {
        match self {
            Bind::PushToTalk => &hotkeys.push_to_talk,
            Bind::HandsFree => &hotkeys.hands_free,
            Bind::Command => &hotkeys.command,
            Bind::PasteLast => &hotkeys.paste_last,
            Bind::CopyLast => &hotkeys.copy_last,
            Bind::Cancel => &hotkeys.cancel,
        }
    }

    /// Whether this shortcut is made only of modifiers, so that a press may
    /// still turn out to be typing for the first `UNSURE_MS` after it goes
    /// down.
    pub fn unsure(self, hotkeys: &speechify_core::settings::Hotkeys) -> bool {
        Shortcut::parse(self.shortcut(hotkeys)).is_ok_and(|shortcut| shortcut.chord.modifiers_only())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Down(Bind),
    Up(Bind),
    /// Another key joined a modifier-only shortcut just after it went down:
    /// it was the start of a capital letter or some other shortcut, not a
    /// dictation.
    Abort(Bind),
    /// Every key is up again; deferred pastes are safe now.
    Released,
}

/// A modifier-only shortcut held alone this long goes down. Typing a capital
/// usually adds the letter sooner, so Shift+A never opens the microphone.
pub const SETTLE_MS: u64 = 150;
/// Another key joining a modifier-only shortcut this soon after it was
/// pressed aborts it. Later, extra keys are ignored, as for any shortcut.
pub const ABORT_MS: u64 = crate::session::TAP_MS;
/// How long after going down such a shortcut may still be aborted.
pub const UNSURE_MS: u64 = ABORT_MS - SETTLE_MS;
const TAP_MS: u64 = crate::session::TAP_MS;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Up,
    /// Modifier-only and held alone since `since`, not yet down.
    Settling {
        since: u64,
    },
    Down {
        since: u64,
    },
    /// Used for something else; waits for its keys to be let go.
    Spent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tap {
    Waiting,
    First { since: u64 },
    Tapped { at: u64 },
    Spoiled,
}

struct Binding {
    bind: Bind,
    chord: Chord,
    double: bool,
    phase: Phase,
    tap: Tap,
}

impl Binding {
    fn second_press(&mut self, held: bool, clean: bool, now: u64) -> bool {
        let tap = match self.tap {
            Tap::Tapped { at } if now.saturating_sub(at) >= TAP_MS => Tap::Waiting,
            tap => tap,
        };
        let (tap, second) = match tap {
            Tap::Tapped { .. } if held => (Tap::Waiting, true),
            Tap::Tapped { .. } if clean => (tap, false),
            Tap::First { since } if !held && clean && now.saturating_sub(since) < TAP_MS => {
                (Tap::Tapped { at: now }, false)
            }
            _ if !held => (Tap::Waiting, false),
            Tap::First { .. } if clean => (tap, false),
            Tap::Waiting if clean => (Tap::First { since: now }, false),
            _ => (Tap::Spoiled, false),
        };
        self.tap = tap;
        second
    }
}

pub struct Tracker {
    bindings: Vec<Binding>,
    any_down: bool,
}

impl Tracker {
    pub fn new(hotkeys: &speechify_core::settings::Hotkeys) -> Result<Tracker> {
        let mut bindings = Vec::new();
        for bind in Bind::ALL {
            let text = bind.shortcut(hotkeys);
            // An empty shortcut switches the action off.
            if !text.trim().is_empty() {
                let Shortcut { chord, double } = Shortcut::parse(text)?;
                bindings.push(Binding {
                    bind,
                    chord,
                    double,
                    // Keys already held when the shortcuts change were
                    // pressed for something else.
                    phase: Phase::Spent,
                    tap: Tap::Waiting,
                });
            }
        }
        bindings.sort_by_key(|binding| !binding.double);
        Ok(Tracker {
            bindings,
            any_down: false,
        })
    }

    /// The shortcuts down now.
    pub fn down(&self) -> Vec<Bind> {
        self.bindings
            .iter()
            .filter(|binding| matches!(binding.phase, Phase::Down { .. }))
            .map(|binding| binding.bind)
            .collect()
    }

    /// Keys held now were pressed for something else, such as recording a
    /// new shortcut: nothing goes down until they are let go.
    pub fn hold_off(&mut self) {
        for binding in &mut self.bindings {
            binding.phase = Phase::Spent;
            binding.tap = Tap::Waiting;
        }
    }

    /// Whether a shortcut is settling, so `update` must be called again as
    /// time passes even though no key changed.
    pub fn settling(&self) -> bool {
        self.bindings
            .iter()
            .any(|binding| matches!(binding.phase, Phase::Settling { .. }))
    }

    /// Feeds the current key state and returns what changed. `now` is a
    /// monotonic clock in milliseconds.
    pub fn update(&mut self, pressed: &BTreeSet<Key>, now: u64) -> Vec<(Edge, u64)> {
        let mut edges = Vec::new();
        let chords: Vec<(Bind, Chord)> = self
            .bindings
            .iter()
            .map(|binding| (binding.bind, binding.chord.clone()))
            .collect();
        let mut doubled: Vec<Chord> = Vec::new();
        for binding in &mut self.bindings {
            let bind = binding.bind;
            let held = binding.chord.held_by(pressed);
            let clean = binding.chord.covers(pressed);
            let mut second = false;
            if binding.phase == Phase::Up {
                if binding.double {
                    if !binding.second_press(held, clean, now) {
                        continue;
                    }
                    second = true;
                } else if held && doubled.iter().any(|keys| binding.chord.held_by(&keys.0)) {
                    binding.phase = Phase::Spent;
                    continue;
                }
            }
            let chord = &binding.chord;
            // Extra keys are fine only when they extend this shortcut into
            // one that carries on dictating (Ctrl+Meta → Ctrl+Meta+Space).
            // Ctrl+Meta+Left is somebody else's shortcut, and Alt+Shift+Z
            // pastes: neither may start dictation.
            let ours = chords.iter().any(|(other_bind, other)| {
                (*other_bind == bind || matches!(other_bind, Bind::HandsFree | Bind::Command))
                    && other.extends(chord)
                    && other.covers(pressed)
            });
            let foreign = held && !ours;
            let extended = held && !foreign && !chord.covers(pressed);
            let quiet = chord.modifiers_only();
            binding.phase = match binding.phase {
                Phase::Up | Phase::Spent if !held => Phase::Up,
                Phase::Spent => Phase::Spent,
                Phase::Up if foreign => {
                    if quiet {
                        Phase::Spent
                    } else {
                        Phase::Up
                    }
                }
                Phase::Up if quiet && !extended => Phase::Settling { since: now },
                Phase::Up => {
                    edges.push((Edge::Down(bind), now));
                    Phase::Down { since: now }
                }
                Phase::Settling { since } if !held => {
                    // Let go cleanly before settling: a tap, which counts.
                    // Let go as another key went down: typing.
                    if chord.covers(pressed) {
                        edges.push((Edge::Down(bind), since));
                        edges.push((Edge::Up(bind), now));
                    }
                    Phase::Up
                }
                Phase::Settling { .. } if foreign => Phase::Spent,
                Phase::Settling { since } if extended || now.saturating_sub(since) >= SETTLE_MS => {
                    edges.push((Edge::Down(bind), since));
                    Phase::Down { since }
                }
                settling @ Phase::Settling { .. } => settling,
                Phase::Down { since } if !held => {
                    // Let go as a letter goes down, still in the abort window:
                    // typing, as when the letter joins first. Space left held
                    // from Shift+Space hands-free is not a letter.
                    let typed = quiet && !ours && now.saturating_sub(since) < ABORT_MS;
                    edges.push((if typed { Edge::Abort(bind) } else { Edge::Up(bind) }, now));
                    Phase::Up
                }
                Phase::Down { since } if quiet && foreign && now.saturating_sub(since) < ABORT_MS => {
                    edges.push((Edge::Abort(bind), now));
                    Phase::Spent
                }
                down @ Phase::Down { .. } => down,
            };
            if second && matches!(binding.phase, Phase::Settling { .. } | Phase::Down { .. }) {
                doubled.push(binding.chord.clone());
            }
        }
        if self.any_down && pressed.is_empty() {
            edges.push((Edge::Released, now));
        }
        self.any_down = !pressed.is_empty();
        edges
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use speechify_core::settings::Hotkeys;
    use Key::*;

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

    fn right_shift() -> Hotkeys {
        Hotkeys {
            push_to_talk: "Right Shift".into(),
            hands_free: String::new(),
            ..windows()
        }
    }

    impl Tracker {
        fn edges(&mut self, pressed: &BTreeSet<Key>, now: u64) -> Vec<Edge> {
            self.update(pressed, now).into_iter().map(|(edge, _)| edge).collect()
        }
    }

    /// A tracker that has seen the keyboard at rest.
    fn fresh(hotkeys: &Hotkeys) -> Tracker {
        let mut tracker = Tracker::new(hotkeys).unwrap();
        assert_eq!(tracker.edges(&keys(&[]), 0), vec![]);
        tracker
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
        for name in ["Right Shift", "rightshift", "right_shift"] {
            assert_eq!(Chord::parse(name).unwrap().to_string(), "Right Shift", "{name}");
        }
        assert_eq!(
            Chord::parse("left ctrl+right option").unwrap().to_string(),
            "Left Ctrl+Right Alt"
        );
        assert_eq!(Chord::parse("Left Cmd+Space").unwrap().to_string(), "Left Meta+Space");
        assert_eq!(Chord::parse("Left Win").unwrap().to_string(), "Left Meta");
        assert_eq!(Chord::parse("right meta").unwrap().to_string(), "Right Meta");
        assert_eq!(Chord::parse("Left Space").unwrap_err(), "Unknown key \"Left Space\"");
        assert_eq!(Chord::parse("Right").unwrap_err(), "Unknown key \"Right\"");
        assert_eq!(Key::from_name("Meta"), Meta);
        assert_eq!(Key::from_name("Right Shift"), RightShift);
        assert_eq!(Key::from_name("Left Alt"), LeftAlt);
        assert_eq!(Key::from_name("Other"), Other);
        for key in [
            LeftCtrl, RightCtrl, LeftAlt, RightAlt, LeftShift, RightShift, LeftMeta, RightMeta,
        ] {
            assert_eq!(Key::from_name(&key.name()), key);
        }
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
    fn either_side_holds_a_plain_modifier() {
        let mut tracker = fresh(&windows());
        tracker.edges(&keys(&[RightCtrl, LeftMeta]), 0);
        assert_eq!(
            tracker.edges(&keys(&[RightCtrl, LeftMeta]), 200),
            vec![Edge::Down(Bind::PushToTalk)]
        );
        assert_eq!(
            tracker.edges(&keys(&[LeftAlt, RightShift, Char('Z')]), 3000),
            vec![Edge::Up(Bind::PushToTalk), Edge::Down(Bind::PasteLast)]
        );
    }

    #[test]
    fn hold_settles_then_release() {
        let mut tracker = fresh(&windows());
        assert_eq!(tracker.edges(&keys(&[Ctrl]), 0), vec![]);
        assert_eq!(tracker.edges(&keys(&[Ctrl, Meta]), 10), vec![]);
        assert!(tracker.settling());
        assert_eq!(tracker.edges(&keys(&[Ctrl, Meta]), 100), vec![]);
        assert_eq!(
            tracker.edges(&keys(&[Ctrl, Meta]), 10 + SETTLE_MS),
            vec![Edge::Down(Bind::PushToTalk)]
        );
        assert!(!tracker.settling());
        assert_eq!(tracker.edges(&keys(&[Ctrl, Meta]), 1000), vec![]);
        assert_eq!(tracker.edges(&keys(&[Meta]), 2000), vec![Edge::Up(Bind::PushToTalk)]);
        assert_eq!(tracker.edges(&keys(&[]), 2010), vec![Edge::Released]);
        assert_eq!(tracker.edges(&keys(&[]), 2020), vec![]);
    }

    #[test]
    fn a_clean_tap_counts_before_it_settles() {
        let mut tracker = fresh(&windows());
        tracker.edges(&keys(&[Ctrl, Meta]), 0);
        assert_eq!(
            tracker.edges(&keys(&[Ctrl]), 80),
            vec![Edge::Down(Bind::PushToTalk), Edge::Up(Bind::PushToTalk)]
        );
    }

    #[test]
    fn extending_a_shortcut_keeps_the_first_one_down() {
        let mut tracker = fresh(&windows());
        tracker.edges(&keys(&[Ctrl, Meta]), 0);
        tracker.edges(&keys(&[Ctrl, Meta]), 200);
        assert_eq!(
            tracker.edges(&keys(&[Ctrl, Meta, Space]), 300),
            vec![Edge::Down(Bind::HandsFree)]
        );
        assert_eq!(
            tracker.edges(&keys(&[]), 400),
            vec![Edge::Up(Bind::PushToTalk), Edge::Up(Bind::HandsFree), Edge::Released]
        );
        // Extended at once, push-to-talk goes down without settling, then
        // Command Mode once it has settled.
        assert_eq!(
            tracker.edges(&keys(&[Ctrl, Meta, Space]), 1000),
            vec![Edge::Down(Bind::PushToTalk), Edge::Down(Bind::HandsFree)]
        );
        tracker.edges(&keys(&[]), 1100);
        assert_eq!(
            tracker.edges(&keys(&[Ctrl, Meta, Alt]), 2000),
            vec![Edge::Down(Bind::PushToTalk)]
        );
        assert_eq!(
            tracker.edges(&keys(&[Ctrl, Meta, Alt]), 2000 + SETTLE_MS),
            vec![Edge::Down(Bind::Command)]
        );
    }

    #[test]
    fn other_apps_shortcuts_do_not_trigger() {
        let mut tracker = fresh(&windows());
        assert_eq!(tracker.edges(&keys(&[Ctrl, Meta, Char('D')]), 0), vec![]);
        // Letting go of the extra key does not start it either: this press was for something else.
        assert_eq!(tracker.edges(&keys(&[Ctrl, Meta]), 50), vec![]);
        assert_eq!(tracker.edges(&keys(&[Ctrl, Meta]), 1000), vec![]);
        assert_eq!(tracker.edges(&keys(&[]), 1100), vec![Edge::Released]);
        // A key pressed once dictation is under way does not interrupt it.
        tracker.edges(&keys(&[Ctrl, Meta]), 2000);
        tracker.edges(&keys(&[Ctrl, Meta]), 2000 + SETTLE_MS);
        assert_eq!(tracker.edges(&keys(&[Ctrl, Meta, Char('D')]), 2000 + ABORT_MS), vec![]);
        assert_eq!(
            tracker.edges(&keys(&[Escape]), 3000),
            vec![Edge::Up(Bind::PushToTalk), Edge::Down(Bind::Cancel)]
        );
    }

    #[test]
    fn a_key_joining_soon_after_aborts() {
        let mut tracker = fresh(&windows());
        tracker.edges(&keys(&[Ctrl, Meta]), 0);
        assert_eq!(
            tracker.edges(&keys(&[Ctrl, Meta]), SETTLE_MS),
            vec![Edge::Down(Bind::PushToTalk)]
        );
        assert_eq!(
            tracker.edges(&keys(&[Ctrl, Meta, Char('D')]), 300),
            vec![Edge::Abort(Bind::PushToTalk)]
        );
        assert_eq!(tracker.edges(&keys(&[Ctrl, Meta]), 400), vec![]);
        assert_eq!(tracker.edges(&keys(&[Ctrl, Meta]), 2000), vec![]);
        assert_eq!(tracker.edges(&keys(&[]), 2100), vec![Edge::Released]);
    }

    #[test]
    fn extra_keys_must_belong_to_a_single_extension() {
        let mut tracker = fresh(&windows());
        assert!(tracker.edges(&keys(&[Ctrl, Meta, Space, Alt]), 0).is_empty());
        assert_eq!(
            tracker.edges(&keys(&[Ctrl, Meta, Space]), 50),
            vec![Edge::Down(Bind::HandsFree)]
        );
    }

    #[test]
    fn right_shift_only() {
        let mut tracker = fresh(&right_shift());
        // Left Shift is for typing.
        assert_eq!(tracker.edges(&keys(&[LeftShift]), 0), vec![]);
        assert!(!tracker.settling());
        assert_eq!(tracker.edges(&keys(&[LeftShift]), 1000), vec![]);
        tracker.edges(&keys(&[]), 1100);
        // Right Shift held alone dictates.
        tracker.edges(&keys(&[RightShift]), 2000);
        assert_eq!(
            tracker.edges(&keys(&[RightShift]), 2000 + SETTLE_MS),
            vec![Edge::Down(Bind::PushToTalk)]
        );
        assert_eq!(
            tracker.edges(&keys(&[]), 5000),
            vec![Edge::Up(Bind::PushToTalk), Edge::Released]
        );
        // Both Shift keys together are not the shortcut.
        assert_eq!(tracker.edges(&keys(&[LeftShift, RightShift]), 6000), vec![]);
        assert_eq!(tracker.edges(&keys(&[RightShift]), 7000), vec![]);
        tracker.edges(&keys(&[]), 7100);
    }

    #[test]
    fn typing_with_right_shift_never_dictates() {
        let mut tracker = fresh(&right_shift());
        // A capital letter: the letter follows within the settling time.
        assert_eq!(tracker.edges(&keys(&[RightShift]), 0), vec![]);
        assert_eq!(tracker.edges(&keys(&[RightShift, Char('A')]), 80), vec![]);
        assert_eq!(tracker.edges(&keys(&[RightShift]), 150), vec![]);
        // More capitals while Shift stays down.
        assert_eq!(tracker.edges(&keys(&[RightShift]), 600), vec![]);
        assert_eq!(tracker.edges(&keys(&[RightShift, Char('B')]), 700), vec![]);
        assert_eq!(tracker.edges(&keys(&[]), 800), vec![Edge::Released]);
        // A slow capital starts and is aborted at once.
        tracker.edges(&keys(&[RightShift]), 1000);
        assert_eq!(
            tracker.edges(&keys(&[RightShift]), 1200),
            vec![Edge::Down(Bind::PushToTalk)]
        );
        assert_eq!(
            tracker.edges(&keys(&[RightShift, Char('C')]), 1250),
            vec![Edge::Abort(Bind::PushToTalk)]
        );
        assert_eq!(tracker.edges(&keys(&[]), 1300), vec![Edge::Released]);
        // Alt+Shift+Z with the right Shift pastes and does not dictate.
        assert_eq!(
            tracker.edges(&keys(&[LeftAlt, RightShift, Char('Z')]), 2000),
            vec![Edge::Down(Bind::PasteLast)]
        );
    }

    #[test]
    fn double_tap_right_shift() {
        let mut tracker = fresh(&right_shift());
        let tap = vec![Edge::Down(Bind::PushToTalk), Edge::Up(Bind::PushToTalk), Edge::Released];
        tracker.edges(&keys(&[RightShift]), 0);
        assert_eq!(tracker.edges(&keys(&[]), 90), tap);
        tracker.edges(&keys(&[RightShift]), 200);
        assert_eq!(tracker.edges(&keys(&[]), 290), tap);
    }

    #[test]
    fn keys_held_when_shortcuts_change_wait_to_be_let_go() {
        let mut tracker = Tracker::new(&right_shift()).unwrap();
        assert_eq!(tracker.edges(&keys(&[RightShift]), 0), vec![]);
        assert_eq!(tracker.edges(&keys(&[RightShift]), 1000), vec![]);
        assert_eq!(tracker.edges(&keys(&[]), 1100), vec![Edge::Released]);
        tracker.edges(&keys(&[RightShift]), 2000);
        assert!(tracker.settling());
        // Recording a shortcut in Settings ends with Right Shift still down.
        assert_eq!(tracker.down(), vec![]);
        tracker.hold_off();
        assert!(!tracker.settling());
        assert_eq!(tracker.edges(&keys(&[RightShift]), 3000), vec![]);
        tracker.edges(&keys(&[]), 3100);
        tracker.edges(&keys(&[RightShift]), 4000);
        assert_eq!(
            tracker.edges(&keys(&[RightShift]), 4000 + SETTLE_MS),
            vec![Edge::Down(Bind::PushToTalk)]
        );
        assert_eq!(tracker.down(), vec![Bind::PushToTalk]);
    }

    #[test]
    fn a_plain_shortcut_extends_one_side() {
        let mut tracker = fresh(&Hotkeys {
            hands_free: "Shift+Space".into(),
            ..right_shift()
        });
        tracker.edges(&keys(&[RightShift]), 0);
        assert_eq!(
            tracker.edges(&keys(&[RightShift, Space]), 50),
            vec![Edge::Down(Bind::PushToTalk), Edge::Down(Bind::HandsFree)]
        );
        // Letting go of Shift first is the end of the press, not typing.
        assert_eq!(
            tracker.edges(&keys(&[Space]), 150),
            vec![Edge::Up(Bind::PushToTalk), Edge::Up(Bind::HandsFree)]
        );
    }

    #[test]
    fn letting_go_as_a_letter_goes_down_soon_after_is_typing() {
        let mut tracker = fresh(&right_shift());
        tracker.edges(&keys(&[RightShift]), 0);
        tracker.edges(&keys(&[RightShift]), SETTLE_MS);
        assert_eq!(
            tracker.edges(&keys(&[Char('A')]), 300),
            vec![Edge::Abort(Bind::PushToTalk)]
        );
        // Later it is just the end of the dictation.
        tracker.edges(&keys(&[]), 400);
        tracker.edges(&keys(&[RightShift]), 1000);
        tracker.edges(&keys(&[RightShift]), 1000 + SETTLE_MS);
        assert_eq!(
            tracker.edges(&keys(&[Char('A')]), 1000 + ABORT_MS),
            vec![Edge::Up(Bind::PushToTalk)]
        );
    }

    #[test]
    fn letting_go_as_a_letter_goes_down_is_typing() {
        let mut tracker = fresh(&right_shift());
        tracker.edges(&keys(&[RightShift]), 0);
        assert_eq!(tracker.edges(&keys(&[Char('A')]), 60), vec![]);
        assert_eq!(tracker.edges(&keys(&[]), 120), vec![Edge::Released]);
    }

    fn double_right_shift() -> Hotkeys {
        Hotkeys {
            hands_free: "Right Shift x2".into(),
            ..windows()
        }
    }

    #[test]
    fn parses_a_double_tap() {
        for text in ["Right Shift x2", " right shift X2 ", "Right Shift  x2"] {
            let shortcut = Shortcut::parse(text).unwrap();
            assert!(shortcut.double, "{text}");
            assert_eq!(shortcut.to_string(), "Right Shift x2", "{text}");
        }
        assert_eq!(Shortcut::parse("Ctrl+Win x2").unwrap().to_string(), "Ctrl+Meta x2");
        let single = Shortcut::parse("Alt+Shift+Z").unwrap();
        assert!(!single.double);
        assert_eq!(single.to_string(), "Alt+Shift+Z");
        assert_eq!(
            Shortcut::parse("Right Shiftx2").unwrap_err(),
            "Unknown key \"Right Shiftx2\""
        );
        assert_eq!(
            Shortcut::parse("Z x2").unwrap_err(),
            "A shortcut needs Ctrl, Alt, Shift or the Windows/Command key"
        );
        assert!(Bind::HandsFree.unsure(&double_right_shift()));
    }

    #[test]
    fn a_double_tap_goes_down_on_its_second_press() {
        let mut tracker = fresh(&double_right_shift());
        assert_eq!(tracker.edges(&keys(&[RightShift]), 1000), vec![]);
        assert_eq!(tracker.edges(&keys(&[RightShift]), 1050), vec![]);
        assert_eq!(tracker.edges(&keys(&[]), 1090), vec![Edge::Released]);
        assert_eq!(tracker.edges(&keys(&[]), 1100), vec![]);
        assert_eq!(tracker.edges(&keys(&[RightShift]), 1300), vec![]);
        assert!(tracker.settling());
        assert_eq!(
            tracker.update(&keys(&[RightShift]), 1300 + SETTLE_MS),
            vec![(Edge::Down(Bind::HandsFree), 1300)]
        );
        assert_eq!(
            tracker.update(&keys(&[]), 3000),
            vec![(Edge::Up(Bind::HandsFree), 3000), (Edge::Released, 3000)]
        );
        tracker.edges(&keys(&[RightShift]), 4000);
        tracker.edges(&keys(&[]), 4080);
        tracker.edges(&keys(&[RightShift]), 4200);
        assert_eq!(
            tracker.update(&keys(&[]), 4280),
            vec![
                (Edge::Down(Bind::HandsFree), 4200),
                (Edge::Up(Bind::HandsFree), 4280),
                (Edge::Released, 4280)
            ]
        );
        tracker.edges(&keys(&[RightShift]), 4400);
        assert_eq!(tracker.edges(&keys(&[]), 4480), vec![Edge::Released]);
    }

    #[test]
    fn only_two_quick_clean_taps_are_a_double_tap() {
        let mut tracker = fresh(&double_right_shift());
        let second_tap = |tracker: &mut Tracker, at: u64| {
            tracker.edges(&keys(&[RightShift]), at);
            tracker.edges(&keys(&[]), at + 80)
        };
        tracker.edges(&keys(&[RightShift]), 0);
        tracker.edges(&keys(&[]), TAP_MS);
        assert_eq!(second_tap(&mut tracker, TAP_MS + 100), vec![Edge::Released]);
        tracker.edges(&keys(&[]), 2000);
        tracker.edges(&keys(&[RightShift]), 3000);
        tracker.edges(&keys(&[]), 3080);
        assert_eq!(second_tap(&mut tracker, 3080 + TAP_MS), vec![Edge::Released]);
        assert_eq!(
            second_tap(&mut tracker, 3080 + TAP_MS + 200),
            vec![Edge::Down(Bind::HandsFree), Edge::Up(Bind::HandsFree), Edge::Released]
        );
        tracker.edges(&keys(&[RightShift]), 5000);
        tracker.edges(&keys(&[RightShift, Char('A')]), 5050);
        tracker.edges(&keys(&[RightShift]), 5080);
        tracker.edges(&keys(&[]), 5100);
        assert_eq!(second_tap(&mut tracker, 5200), vec![Edge::Released]);
        tracker.edges(&keys(&[]), 6000);
        tracker.edges(&keys(&[RightShift]), 6100);
        tracker.edges(&keys(&[Char('A')]), 6150);
        tracker.edges(&keys(&[]), 6200);
        assert_eq!(second_tap(&mut tracker, 6300), vec![Edge::Released]);
        tracker.edges(&keys(&[]), 7000);
        tracker.edges(&keys(&[RightShift]), 7100);
        tracker.edges(&keys(&[]), 7180);
        tracker.edges(&keys(&[RightShift]), 7300);
        assert_eq!(tracker.edges(&keys(&[RightShift, Char('A')]), 7350), vec![]);
        assert_eq!(tracker.edges(&keys(&[]), 7400), vec![Edge::Released]);
    }

    #[test]
    fn a_double_tap_of_the_push_to_talk_key_is_not_also_a_press_of_it() {
        let mut tracker = fresh(&Hotkeys {
            push_to_talk: "Right Shift".into(),
            ..double_right_shift()
        });
        tracker.edges(&keys(&[RightShift]), 0);
        assert_eq!(
            tracker.edges(&keys(&[]), 80),
            vec![Edge::Down(Bind::PushToTalk), Edge::Up(Bind::PushToTalk), Edge::Released]
        );
        tracker.edges(&keys(&[RightShift]), 200);
        assert_eq!(
            tracker.edges(&keys(&[RightShift]), 200 + SETTLE_MS),
            vec![Edge::Down(Bind::HandsFree)]
        );
        assert_eq!(
            tracker.edges(&keys(&[]), 2000),
            vec![Edge::Up(Bind::HandsFree), Edge::Released]
        );
        tracker.edges(&keys(&[RightShift]), 3000);
        assert_eq!(
            tracker.edges(&keys(&[RightShift]), 3000 + SETTLE_MS),
            vec![Edge::Down(Bind::PushToTalk)]
        );
    }

    #[test]
    fn typing_between_the_taps_is_not_a_double_tap() {
        let mut tracker = fresh(&double_right_shift());
        tracker.edges(&keys(&[RightShift]), 0);
        tracker.edges(&keys(&[]), 80);
        tracker.edges(&keys(&[Char('A')]), 100);
        tracker.edges(&keys(&[]), 120);
        tracker.edges(&keys(&[RightShift]), 200);
        assert_eq!(tracker.edges(&keys(&[]), 280), vec![Edge::Released]);
    }

    #[test]
    fn a_rejected_second_tap_leaves_other_shortcuts_alone() {
        let mut tracker = fresh(&double_right_shift());
        tracker.edges(&keys(&[RightShift]), 0);
        tracker.edges(&keys(&[]), 80);
        assert_eq!(
            tracker.edges(&keys(&[LeftAlt, RightShift, Char('Z')]), 200),
            vec![Edge::Down(Bind::PasteLast)]
        );
    }

    #[test]
    fn holding_off_forgets_a_first_tap() {
        let mut tracker = fresh(&double_right_shift());
        tracker.edges(&keys(&[RightShift]), 0);
        tracker.edges(&keys(&[]), 80);
        tracker.hold_off();
        tracker.edges(&keys(&[]), 90);
        tracker.edges(&keys(&[RightShift]), 200);
        assert_eq!(tracker.edges(&keys(&[]), 280), vec![Edge::Released]);
    }

    #[test]
    fn unsure_means_modifiers_only() {
        let hotkeys = right_shift();
        assert!(Bind::PushToTalk.unsure(&hotkeys));
        assert!(Bind::Command.unsure(&hotkeys));
        assert!(!Bind::HandsFree.unsure(&hotkeys));
        assert!(!Bind::PasteLast.unsure(&hotkeys));
        assert!(!Bind::Cancel.unsure(&hotkeys));
        assert!(!Bind::CopyLast.unsure(&hotkeys));
    }
}
