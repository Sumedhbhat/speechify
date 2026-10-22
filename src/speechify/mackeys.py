"""Keyboard input and output on macOS through Quartz event taps.

pynput's listener looks up the keyboard layout from its own thread, which
newer macOS versions only allow on the main thread, and it reports letters as
Mac key codes the hotkey matcher can't use. This reads physical key codes and
names them with keymaps.MAC instead. Listening needs Input Monitoring
permission and typing needs Accessibility; see permission_problem().
"""

from __future__ import annotations

import threading
from typing import Callable

import Quartz

from . import keymaps

_CAPS_LOCK = 57


def permission_problem(prompt: bool = True) -> str | None:
    """None if we may listen and type; otherwise what the user has to allow.

    With prompt=True, macOS shows its own permission dialog the first time and
    adds Speechify to the list in System Settings.
    """
    missing = []
    preflight_listen = getattr(Quartz, "CGPreflightListenEventAccess", None)
    if preflight_listen and not preflight_listen():
        missing.append("Input Monitoring")
        if prompt:
            Quartz.CGRequestListenEventAccess()
    preflight_post = getattr(Quartz, "CGPreflightPostEventAccess", None)
    if preflight_post and not preflight_post():
        missing.append("Accessibility")
        if prompt:
            Quartz.CGRequestPostEventAccess()
    if not missing:
        return None
    return (
        f"Allow Speechify under System Settings > Privacy & Security > {' and '.join(missing)}, "
        "then restart it. (If you started it from a terminal, allow the terminal app instead.)"
    )


class Listener:
    """Calls on_press / on_release(name, vk, char) for every key, like pynput."""

    def __init__(self, on_press: Callable, on_release: Callable):
        self.on_press, self.on_release = on_press, on_release
        self._tap = None
        self._loop = None
        self._ready = threading.Event()
        self._thread = threading.Thread(target=self._run, daemon=True, name="quartz-keys")

    def start(self) -> None:
        self._thread.start()
        self._ready.wait(5)
        if self._tap is None:
            raise PermissionError(permission_problem(prompt=False) or "Couldn't watch the keyboard.")

    def stop(self) -> None:
        if self._loop is not None:
            Quartz.CFRunLoopStop(self._loop)

    def _run(self) -> None:
        mask = (Quartz.CGEventMaskBit(Quartz.kCGEventKeyDown)
                | Quartz.CGEventMaskBit(Quartz.kCGEventKeyUp)
                | Quartz.CGEventMaskBit(Quartz.kCGEventFlagsChanged))
        self._tap = Quartz.CGEventTapCreate(
            Quartz.kCGSessionEventTap, Quartz.kCGHeadInsertEventTap,
            Quartz.kCGEventTapOptionListenOnly, mask, self._callback, None,
        )
        if self._tap is None:  # no Input Monitoring permission
            self._ready.set()
            return
        source = Quartz.CFMachPortCreateRunLoopSource(None, self._tap, 0)
        self._loop = Quartz.CFRunLoopGetCurrent()
        Quartz.CFRunLoopAddSource(self._loop, source, Quartz.kCFRunLoopCommonModes)
        Quartz.CGEventTapEnable(self._tap, True)
        self._ready.set()
        Quartz.CFRunLoopRun()

    def _callback(self, proxy, kind, event, refcon):
        try:
            if kind in (Quartz.kCGEventTapDisabledByTimeout, Quartz.kCGEventTapDisabledByUserInput):
                Quartz.CGEventTapEnable(self._tap, True)  # macOS turns slow taps off
                return event
            code = Quartz.CGEventGetIntegerValueField(event, Quartz.kCGKeyboardEventKeycode)
            name, char = keymaps.MAC.get(code, (None, None))
            if name is None and char is None:
                return event
            if kind == Quartz.kCGEventKeyDown:
                if not Quartz.CGEventGetIntegerValueField(event, Quartz.kCGKeyboardEventAutorepeat):
                    self.on_press(name, None, char)
            elif kind == Quartz.kCGEventKeyUp:
                self.on_release(name, None, char)
            elif kind == Quartz.kCGEventFlagsChanged:
                if code == _CAPS_LOCK:  # reported once per toggle, not per press
                    self.on_press(name, None, char)
                    self.on_release(name, None, char)
                elif code in keymaps.MAC_MODIFIER_BITS:
                    down = Quartz.CGEventGetFlags(event) & keymaps.MAC_MODIFIER_BITS[code]
                    (self.on_press if down else self.on_release)(name, None, char)
        except Exception as exc:  # an exception here would kill the event tap
            print(f"key handler failed: {exc}")
        return event


def _post(code: int, down: bool, flags: int) -> None:
    event = Quartz.CGEventCreateKeyboardEvent(None, code, down)
    Quartz.CGEventSetFlags(event, flags)
    Quartz.CGEventPost(Quartz.kCGHIDEventTap, event)


def cmd_v() -> None:
    cmd = Quartz.kCGEventFlagMaskCommand
    _post(keymaps.MAC_CMD, True, cmd)
    _post(keymaps.MAC_V, True, cmd)
    _post(keymaps.MAC_V, False, cmd)
    _post(keymaps.MAC_CMD, False, 0)


def shift_left(count: int) -> None:
    shift = Quartz.kCGEventFlagMaskShift
    for _ in range(count):
        _post(keymaps.MAC_LEFT, True, shift)
        _post(keymaps.MAC_LEFT, False, shift)
