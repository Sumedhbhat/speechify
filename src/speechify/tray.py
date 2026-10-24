"""The tray icon (Windows, Linux) or menu bar icon (macOS): Speechify's home.

Its menu opens every window, shows what's going on, and quits. pystray runs
the icon on its own thread, except on macOS, where it has to share the main
thread's run loop with Tk. Menu callbacks run on pystray's thread, so they
only post work to the UI.
"""

from __future__ import annotations

import sys
import threading
from typing import Callable

from . import icon


class Tray:
    def __init__(
        self,
        status: Callable[[], str],
        hint: Callable[[], str],
        actions: list[tuple[str | Callable[[], str], Callable[[], None]] | None],
    ):
        """`status` and `hint` are the menu's first two lines, read each time
        the menu opens.
        `actions` are (label, callback) pairs, None for a separator; a label
        may be a function, so it follows settings changes. The first action is
        what clicking the icon does."""
        self.status, self.hint, self.actions = status, hint, actions
        self._icon = None
        self._images: dict[bool, object] = {}
        self._listening = False

    def start(self) -> bool:
        """Show the icon (call on the main thread). False if this desktop has
        no tray, or pystray isn't installed; the app works without it."""
        try:
            import pystray

            self._images = {False: icon.draw(64), True: icon.draw(64, icon.LISTENING)}
            items = [
                pystray.MenuItem(lambda item: self.status(), None, enabled=False),
                pystray.MenuItem(lambda item: self.hint(), None, enabled=False),
                pystray.Menu.SEPARATOR,
            ]
            for i, action in enumerate(self.actions):
                if action is None:
                    items.append(pystray.Menu.SEPARATOR)
                else:
                    label, callback = action
                    text = (lambda item, f=label: f()) if callable(label) else label
                    items.append(pystray.MenuItem(text, _ignore_args(callback), default=i == 0))
            self._icon = pystray.Icon("speechify", self._images[False], "Speechify", pystray.Menu(*items))
            if sys.platform == "darwin":
                self._icon.run_detached()
            else:
                threading.Thread(target=self._icon.run, daemon=True, name="tray").start()
            return True
        except Exception as exc:
            print(f"No tray icon ({type(exc).__name__}: {exc}). Start Speechify again to open its settings.")
            self._icon = None
            return False

    def set_listening(self, listening: bool) -> None:
        """Turn the icon red while the talk key is held."""
        if self._icon is not None and listening != self._listening:
            self._listening = listening
            self._icon.icon = self._images[listening]

    def refresh(self) -> None:
        """Re-read the status line (some desktops only read it on change)."""
        if self._icon is not None:
            try:
                self._icon.update_menu()
            except Exception:
                pass

    def stop(self) -> None:
        if self._icon is not None:
            self._icon.stop()
            self._icon = None


def _ignore_args(callback: Callable[[], None]):
    """pystray passes (icon, item); ours take nothing."""
    return lambda *args: callback()
