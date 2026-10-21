"""Global push-to-talk key handling."""

from __future__ import annotations

import threading
import time
from typing import Callable

from pynput import keyboard

from .combos import Matcher


def parse_key(name: str) -> keyboard.Key | keyboard.KeyCode:
    if len(name) == 1:
        return keyboard.KeyCode.from_char(name.lower())
    try:
        return getattr(keyboard.Key, name)
    except AttributeError:
        raise ValueError(f"unknown key {name!r}; use a pynput Key name like 'ctrl_r'") from None


class PushToTalk:
    """Calls on_start when the key goes down and on_stop when it comes up.

    If any other key is pressed while it's held (you were doing Ctrl+C, not
    dictating), or the press was shorter than min_hold_ms, on_cancel is
    called instead of on_stop.
    """

    def __init__(
        self,
        key: str,
        min_hold_ms: int,
        on_start: Callable[[], None],
        on_stop: Callable[[], None],
        on_cancel: Callable[[], None],
    ):
        self.key = parse_key(key)
        self.min_hold = min_hold_ms / 1000
        self.on_start, self.on_stop, self.on_cancel = on_start, on_stop, on_cancel
        self._down_at: float | None = None
        self._interrupted = False
        self._listener = keyboard.Listener(on_press=self._press, on_release=self._release)

    def start(self) -> None:
        self._listener.start()

    def stop(self) -> None:
        self._listener.stop()

    def _matches(self, key) -> bool:
        if isinstance(self.key, keyboard.KeyCode) and isinstance(key, keyboard.KeyCode):
            return (key.char or "").lower() == self.key.char
        return key == self.key

    def _press(self, key) -> None:
        if self._matches(key):
            if self._down_at is None:  # ignore OS key auto-repeat
                self._down_at = time.perf_counter()
                self._interrupted = False
                self.on_start()
        elif self._down_at is not None and not self._interrupted:
            self._interrupted = True
            self.on_cancel()

    def _release(self, key) -> None:
        if not self._matches(key) or self._down_at is None:
            return
        held = time.perf_counter() - self._down_at
        self._down_at = None
        if self._interrupted:
            return
        if held < self.min_hold:
            self.on_cancel()
        else:
            self.on_stop()


def _describe(key) -> tuple[str | None, int | None, str | None]:
    if isinstance(key, keyboard.Key):
        return key.name, getattr(key.value, "vk", None), None
    return None, getattr(key, "vk", None), getattr(key, "char", None)


class GlobalHotkeys:
    """System-wide shortcuts like "<ctrl>+<alt>+f".

    Replaces pynput's GlobalHotKeys, which never fires Ctrl+Alt+letter on
    Windows (see combos.py). Callbacks run on their own thread so a slow one
    can't stall the system keyboard hook.
    """

    def __init__(self, bindings: dict[str, Callable[[], None]]):
        self.bindings = bindings
        self.matcher = Matcher(list(bindings))
        self._listener = keyboard.Listener(on_press=self._press, on_release=self._release)

    def start(self) -> None:
        self._listener.start()

    def stop(self) -> None:
        self._listener.stop()

    def _press(self, key) -> None:
        for combo in self.matcher.press(*_describe(key)):
            threading.Thread(target=self.bindings[combo], daemon=True).start()

    def _release(self, key) -> None:
        self.matcher.release(*_describe(key))


def global_hotkeys(bindings: dict[str, Callable[[], None]]) -> GlobalHotkeys:
    return GlobalHotkeys(bindings)
