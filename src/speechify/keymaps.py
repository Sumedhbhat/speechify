"""Key codes for the native Linux (evdev) and macOS (Quartz) keyboard code.

Both turn a physical key into the same (name, char) pair pynput reports, so
config values like "ctrl_r" and "<ctrl>+<alt>+f" mean the same on every OS.
Letters are named by their position on a US QWERTY keyboard. Kept free of
platform imports so it can be tested anywhere.
"""

from __future__ import annotations

# Linux input event codes (linux/input-event-codes.h). Printable keys map to
# their character, everything else to a pynput Key name.
_EVDEV_CHARS = {
    2: "1", 3: "2", 4: "3", 5: "4", 6: "5", 7: "6", 8: "7", 9: "8", 10: "9", 11: "0",
    12: "-", 13: "=", 16: "q", 17: "w", 18: "e", 19: "r", 20: "t", 21: "y", 22: "u",
    23: "i", 24: "o", 25: "p", 26: "[", 27: "]", 30: "a", 31: "s", 32: "d", 33: "f",
    34: "g", 35: "h", 36: "j", 37: "k", 38: "l", 39: ";", 40: "'", 41: "`", 43: "\\",
    44: "z", 45: "x", 46: "c", 47: "v", 48: "b", 49: "n", 50: "m", 51: ",", 52: ".",
    53: "/",
}
_EVDEV_NAMES = {
    1: "esc", 14: "backspace", 15: "tab", 28: "enter", 29: "ctrl_l", 42: "shift_l",
    54: "shift_r", 56: "alt_l", 57: "space", 58: "caps_lock", 69: "num_lock",
    70: "scroll_lock", 96: "enter", 97: "ctrl_r", 99: "print_screen", 100: "alt_r",
    102: "home", 103: "up", 104: "page_up", 105: "left", 106: "right", 107: "end",
    108: "down", 109: "page_down", 110: "insert", 111: "delete", 119: "pause",
    125: "cmd_l", 126: "cmd_r", 127: "menu",
    **{59 + i: f"f{i + 1}" for i in range(10)},  # F1-F10
    87: "f11", 88: "f12",
    **{183 + i: f"f{i + 13}" for i in range(12)},  # F13-F24
}

# macOS virtual key codes (HIToolbox Events.h, kVK_*).
_MAC_CHARS = {
    0: "a", 1: "s", 2: "d", 3: "f", 4: "h", 5: "g", 6: "z", 7: "x", 8: "c", 9: "v",
    11: "b", 12: "q", 13: "w", 14: "e", 15: "r", 16: "y", 17: "t", 18: "1", 19: "2",
    20: "3", 21: "4", 22: "6", 23: "5", 24: "=", 25: "9", 26: "7", 27: "-", 28: "8",
    29: "0", 30: "]", 31: "o", 32: "u", 33: "[", 34: "i", 35: "p", 37: "l", 38: "j",
    39: "'", 40: "k", 41: ";", 42: "\\", 43: ",", 44: "/", 45: "n", 46: "m", 47: ".",
    50: "`",
}
_MAC_NAMES = {
    36: "enter", 48: "tab", 49: "space", 51: "backspace", 53: "esc", 54: "cmd_r",
    55: "cmd_l", 56: "shift_l", 57: "caps_lock", 58: "alt_l", 59: "ctrl_l",
    60: "shift_r", 61: "alt_r", 62: "ctrl_r", 63: "fn", 76: "enter", 114: "insert",
    115: "home", 116: "page_up", 117: "delete", 119: "end", 121: "page_down",
    123: "left", 124: "right", 125: "down", 126: "up",
    122: "f1", 120: "f2", 99: "f3", 118: "f4", 96: "f5", 97: "f6", 98: "f7", 100: "f8",
    101: "f9", 109: "f10", 103: "f11", 111: "f12", 105: "f13", 107: "f14", 113: "f15",
    106: "f16", 64: "f17", 79: "f18", 80: "f19", 90: "f20",
}
# Modifier keys arrive as "flags changed" events on macOS. These are the
# device-dependent flag bits (NX_DEVICE*KEYMASK) that tell left from right.
MAC_MODIFIER_BITS = {
    59: 0x00000001, 56: 0x00000002, 60: 0x00000004, 55: 0x00000008,
    54: 0x00000010, 58: 0x00000020, 61: 0x00000040, 62: 0x00002000,
    63: 0x00800000,  # fn (kCGEventFlagMaskSecondaryFn)
}

EVDEV = {**{k: (None, c) for k, c in _EVDEV_CHARS.items()},
         **{k: (n, None) for k, n in _EVDEV_NAMES.items()}}
MAC = {**{k: (None, c) for k, c in _MAC_CHARS.items()},
       **{k: (n, None) for k, n in _MAC_NAMES.items()}}

# Key names each native backend can report, for checking the talk key.
# Generic names match the left key (combos.same_key). Caps Lock can't be held
# on macOS: it only reports each toggle.
_GENERIC = frozenset({"ctrl", "alt", "shift", "cmd", "alt_gr"})
EVDEV_NAMES = frozenset(_EVDEV_NAMES.values()) | _GENERIC
MAC_NAMES = (frozenset(_MAC_NAMES.values()) - {"caps_lock"}) | _GENERIC

# evdev codes the Linux virtual keyboard sends.
EV_LEFTCTRL, EV_LEFTSHIFT, EV_V, EV_LEFT = 29, 42, 47, 105
# macOS codes the paste and replace shortcuts send.
MAC_CMD, MAC_V, MAC_LEFT = 55, 9, 123
