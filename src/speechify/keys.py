"""Which keyboard backend this machine uses, and a listener for it.

    Windows          pynput (the low-level keyboard hook) + winkeys.py to type
    macOS            mackeys.py (Quartz event taps)
    Linux on X11     pynput (XRecord / XTest)
    Linux on Wayland linuxkeys.py (evdev / uinput); X11 can't see Wayland windows

Every listener calls on_press / on_release(name, vk, char): a pynput-style
key name such as "ctrl_r", or the character for printable keys.
SPEECHIFY_KEYBOARD=pynput|evdev overrides the choice on Linux.
"""

from __future__ import annotations

import os
import sys
from functools import cache
from typing import Callable

from . import keymaps

Callback = Callable[[str | None, int | None, str | None], None]


@cache
def backend() -> str:
    if sys.platform == "win32":
        return "pynput"
    if sys.platform == "darwin":
        return "quartz"
    choice = os.environ.get("SPEECHIFY_KEYBOARD", "")
    if choice in ("pynput", "evdev"):
        return choice
    wayland = os.environ.get("WAYLAND_DISPLAY") or os.environ.get("XDG_SESSION_TYPE") == "wayland"
    return "evdev" if wayland else "pynput"


def check_key_name(name: str) -> str:
    """Validate a push-to-talk key from the config; returns it normalised."""
    if len(name) == 1:
        return name.lower()
    if backend() == "pynput":
        from pynput import keyboard

        if hasattr(keyboard.Key, name):
            return name
    elif name in (keymaps.MAC_NAMES if backend() == "quartz" else keymaps.EVDEV_NAMES):
        return name
    raise ValueError(f"{name!r} can't be the talk key here; use a key name like 'ctrl_r', 'alt_r' or 'f13'")


class _PynputListener:
    def __init__(self, on_press: Callback, on_release: Callback):
        from pynput import keyboard

        def describe(key):
            if isinstance(key, keyboard.Key):
                return key.name, getattr(key.value, "vk", None), None
            return None, getattr(key, "vk", None), getattr(key, "char", None)

        self._listener = keyboard.Listener(
            on_press=lambda key: on_press(*describe(key)),
            on_release=lambda key: on_release(*describe(key)),
        )

    def start(self) -> None:
        self._listener.start()

    def stop(self) -> None:
        self._listener.stop()


def listener(on_press: Callback, on_release: Callback):
    kind = backend()
    if kind == "quartz":
        from .mackeys import Listener
    elif kind == "evdev":
        from .linuxkeys import Listener
    else:
        Listener = _PynputListener
    return Listener(on_press, on_release)


def permission_problem() -> str | None:
    """What the user must allow before keys work, if anything (macOS asks once)."""
    if backend() == "quartz":
        from .mackeys import permission_problem

        return permission_problem()
    return None
