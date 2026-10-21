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
        # code; X11 keysyms are the lowercase one.
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
