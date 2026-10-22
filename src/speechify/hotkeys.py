"""Global push-to-talk key handling."""

from __future__ import annotations

import threading
import time
from typing import Callable

from . import keys
from .combos import Matcher, same_key


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
        self.key = keys.check_key_name(key)
        self.min_hold = min_hold_ms / 1000
        self.on_start, self.on_stop, self.on_cancel = on_start, on_stop, on_cancel
        self._down_at: float | None = None
        self._interrupted = False
        self._listener = keys.listener(self._press, self._release)

    def start(self) -> None:
        self._listener.start()

    def stop(self) -> None:
        self._listener.stop()

    def _press(self, name, vk, char) -> None:
        if same_key(self.key, name, char):
            if self._down_at is None:  # ignore OS key auto-repeat
                self._down_at = time.perf_counter()
                self._interrupted = False
                self.on_start()
        elif self._down_at is not None and not self._interrupted:
            self._interrupted = True
            self.on_cancel()

    def _release(self, name, vk, char) -> None:
        if not same_key(self.key, name, char) or self._down_at is None:
            return
        held = time.perf_counter() - self._down_at
        self._down_at = None
        if self._interrupted:
            return
        if held < self.min_hold:
            self.on_cancel()
        else:
            self.on_stop()


class GlobalHotkeys:
    """System-wide shortcuts like "<ctrl>+<alt>+f".

    Replaces pynput's GlobalHotKeys, which never fires Ctrl+Alt+letter on
    Windows (see combos.py). Callbacks run on their own thread so a slow one
    can't stall the system keyboard hook.
    """

    def __init__(self, bindings: dict[str, Callable[[], None]]):
        self.bindings = bindings
        self.matcher = Matcher(list(bindings))
        self._listener = keys.listener(self._press, self.matcher.release)

    def start(self) -> None:
        self._listener.start()

    def stop(self) -> None:
        self._listener.stop()

    def _press(self, name, vk, char) -> None:
        for combo in self.matcher.press(name, vk, char):
            threading.Thread(target=self.bindings[combo], daemon=True).start()


def global_hotkeys(bindings: dict[str, Callable[[], None]]) -> GlobalHotkeys:
    return GlobalHotkeys(bindings)
