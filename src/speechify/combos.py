"""Matching key combinations like "<ctrl>+<alt>+f" against key events.

pynput's GlobalHotKeys compares the character a key produced, but on Windows
a letter pressed with Ctrl+Alt held produces no usable character, so those
hotkeys never fire. This matches the final key by virtual-key code instead,
and is kept free of pynput so it can be tested anywhere.
"""

from __future__ import annotations

MODIFIERS = {
    "ctrl": {"ctrl", "ctrl_l", "ctrl_r"},
    "alt": {"alt", "alt_l", "alt_r", "alt_gr"},
    "shift": {"shift", "shift_l", "shift_r"},
    "cmd": {"cmd", "cmd_l", "cmd_r"},
}
_MOD_OF = {name: mod for mod, names in MODIFIERS.items() for name in names}


# Names that mean the same physical key on different platforms.
_SAME = {"ctrl": "ctrl_l", "alt": "alt_l", "shift": "shift_l", "cmd": "cmd_l", "alt_gr": "alt_r"}


def same_key(want: str, name: str | None, char: str | None) -> bool:
    """Is this key event the configured key? `want` is a key name or one character."""
    if len(want) == 1:
        return (char or "").lower() == want.lower()
    return name is not None and _SAME.get(name, name) == _SAME.get(want, want)


def modifier(key_name: str | None) -> str | None:
    """'ctrl_l' -> 'ctrl'; None for keys that aren't modifiers."""
    return _MOD_OF.get(key_name or "")


def parse(combo: str) -> tuple[frozenset[str], str]:
    """'<ctrl>+<alt>+f' -> ({'ctrl', 'alt'}, 'f')."""
    parts = [p.strip().strip("<>").lower() for p in combo.split("+")]
    *mods, last = parts
    unknown = [m for m in mods if m not in MODIFIERS]
    if unknown or not last:
        raise ValueError(f"can't parse hotkey {combo!r}")
    return frozenset(mods), last


def key_matches(last: str, name: str | None, vk: int | None, char: str | None) -> bool:
    """Does a key event (pynput Key name, or KeyCode vk/char) match `last`?"""
    if len(last) == 1:
        # Letters and digits: Windows virtual-key codes are the uppercase ASCII
        # code; X11 keysyms are the lowercase one. The native Linux and macOS
        # backends report no vk, only the character (see keymaps.py).
        if vk is not None and vk in (ord(last.upper()), ord(last.lower())):
            return True
        return (char or "").lower() == last
    return name == last


class Matcher:
    """Tracks held modifiers and says which combos a key press completes."""

    def __init__(self, combos: list[str]):
        self.combos = [(combo, *parse(combo)) for combo in combos]
        self.held: dict[str, int] = {}  # modifier key name -> held
        self.down: set = set()  # non-modifier keys currently held, to ignore auto-repeat

    @property
    def mods(self) -> frozenset[str]:
        return frozenset(_MOD_OF[name] for name in self.held)

    def press(self, name: str | None, vk: int | None, char: str | None) -> list[str]:
        if modifier(name):
            self.held[name] = 1
            return []
        key_id = (name, vk, (char or "").lower())
        if key_id in self.down:
            return []  # OS auto-repeat while the key is held
        self.down.add(key_id)
        mods = self.mods
        return [combo for combo, want, last in self.combos
                if want == mods and key_matches(last, name, vk, char)]

    def release(self, name: str | None, vk: int | None, char: str | None) -> None:
        if modifier(name):
            self.held.pop(name, None)
        else:
            self.down.discard((name, vk, (char or "").lower()))


# --- names people read -------------------------------------------------------

_MOD_LABELS = {"ctrl": "Ctrl", "alt": "Alt", "shift": "Shift", "cmd": "Win"}
_MAC_MOD_LABELS = {"ctrl": "Control", "alt": "Option", "shift": "Shift", "cmd": "Command"}
_KEY_LABELS = {
    "caps_lock": "Caps Lock", "scroll_lock": "Scroll Lock", "num_lock": "Num Lock",
    "print_screen": "Print Screen", "page_up": "Page Up", "page_down": "Page Down",
    "alt_gr": "AltGr", "esc": "Esc", "menu": "Menu",
}


def describe_key(name: str, mac: bool = False) -> str:
    """'ctrl_r' -> 'Right Ctrl'; 'alt_r' on a Mac -> 'Right Option'."""
    if len(name) == 1:
        return name.upper()
    if name in _KEY_LABELS:
        return _KEY_LABELS[name]
    base, _, side = name.partition("_")
    if base in MODIFIERS and side in ("l", "r", ""):
        label = (_MAC_MOD_LABELS if mac else _MOD_LABELS)[base]
        return {"l": f"Left {label}", "r": f"Right {label}", "": label}[side]
    return name.replace("_", " ").title() if not name[1:].isdigit() else name.upper()


def describe_combo(combo: str, mac: bool = False) -> str:
    """'<ctrl>+<alt>+h' -> 'Ctrl+Alt+H' (Mac: 'Control+Option+H')."""
    mods, last = parse(combo)
    labels = _MAC_MOD_LABELS if mac else _MOD_LABELS
    order = [m for m in ("ctrl", "alt", "shift", "cmd") if m in mods]
    return "+".join([labels[m] for m in order] + [describe_key(last, mac)])


# Tk keysyms for keys that can be the talk key, as config key names.
_TK_KEYS = {
    "Control_L": "ctrl_l", "Control_R": "ctrl_r",
    "Alt_L": "alt_l", "Alt_R": "alt_r", "Option_L": "alt_l", "Option_R": "alt_r",
    "ISO_Level3_Shift": "alt_gr",
    "Shift_L": "shift_l", "Shift_R": "shift_r",
    "Super_L": "cmd_l", "Super_R": "cmd_r", "Win_L": "cmd_l", "Win_R": "cmd_r",
    "Meta_L": "cmd_l", "Meta_R": "cmd_r", "Command": "cmd_l",
    "Caps_Lock": "caps_lock", "Scroll_Lock": "scroll_lock", "Num_Lock": "num_lock",
    "Pause": "pause", "Insert": "insert", "Menu": "menu", "App": "menu", "Print": "print_screen",
}


def key_from_tk(keysym: str) -> str | None:
    """The config name for a key pressed in a Tk window, or None if it
    can't be the talk key (letters would type while you hold them)."""
    if keysym in _TK_KEYS:
        return _TK_KEYS[keysym]
    if keysym[:1] == "F" and keysym[1:].isdigit():
        return keysym.lower()
    return None
