"""Inserting text into whatever app has focus."""

from __future__ import annotations

import sys
import threading
import time

import pyperclip
from pynput.keyboard import Controller, Key

_keyboard = Controller()
_PASTE_MOD = Key.cmd if sys.platform == "darwin" else Key.ctrl


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
    with _keyboard.pressed(_PASTE_MOD):
        _keyboard.tap("v")
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
    with _keyboard.pressed(Key.shift):
        for _ in range(len(old)):
            _keyboard.tap(Key.left)
    paste(new)
