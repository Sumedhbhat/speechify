"""Inserting text into whatever app has focus."""

from __future__ import annotations

import functools
import sys
import threading
import time

import pyperclip

from . import keys

# Windows sends keys with SendInput (pynput's plain "v" is a Unicode character
# event that ignores the held Ctrl), macOS with Quartz, Linux on Wayland
# through a uinput virtual keyboard, and Linux on X11 with pynput (XTest).


def _native():
    if sys.platform == "win32":
        from . import winkeys

        return winkeys
    if sys.platform == "darwin":
        from . import mackeys

        return mackeys
    if keys.backend() == "evdev":
        from . import linuxkeys

        return linuxkeys
    return None


@functools.cache
def _pynput():
    """One pynput controller for X11 (each opens its own X connection)."""
    from pynput.keyboard import Controller, Key

    return Controller(), Key


def prepare() -> None:
    """Set up key output at startup, so the first paste isn't slower or the
    first to find out it's not allowed."""
    if keys.backend() == "evdev":
        from . import linuxkeys

        linuxkeys.keyboard()


def _paste_keys() -> None:
    native = _native()
    if sys.platform == "darwin":
        native.cmd_v()
        return
    if native is not None:
        native.ctrl_v()
        return
    kb, Key = _pynput()
    with kb.pressed(Key.ctrl):
        kb.tap("v")


def _select_left(count: int) -> None:
    native = _native()
    if native is not None:
        native.shift_left(count)
        return
    kb, Key = _pynput()
    with kb.pressed(Key.shift):
        for _ in range(count):
            kb.tap(Key.left)


def paste(text: str) -> None:
    """Paste via the clipboard, then restore what was there.

    Typing character by character is slow and breaks on non-ASCII and on apps
    with autocomplete; a clipboard paste is instant and exact.
    """
    try:
        previous = pyperclip.paste()
    except pyperclip.PyperclipException:
        previous = None
    pyperclip.copy(text)
    time.sleep(0.02)  # let the clipboard owner settle before the target reads it
    _paste_keys()
    if previous is not None:
        # The target app reads the clipboard asynchronously after Ctrl+V.
        threading.Timer(0.4, _restore, args=(text, previous)).start()


def _restore(pasted: str, previous: str) -> None:
    try:
        # Don't clobber something the user copied in the meantime.
        if pyperclip.paste() == pasted:
            pyperclip.copy(previous)
    except pyperclip.PyperclipException:
        pass


def replace_last(old: str, new: str) -> None:
    """Swap text we just pasted for a corrected version.

    Selects backwards over the old text with Shift+Left rather than using
    Ctrl+Z, which suspends processes in terminals and is unpredictable in
    apps that merge undo steps. Assumes the cursor hasn't moved since.
    """
    time.sleep(0.15)  # give focus time to return to the target app
    _select_left(len(old))
    paste(new)
