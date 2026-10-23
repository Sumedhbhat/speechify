"""Keyboard input and output on macOS through Quartz event taps.

pynput's listener looks up the keyboard layout from its own thread, which
newer macOS versions only allow on the main thread, and it reports letters as
Mac key codes the hotkey matcher can't use. This reads physical key codes and
names them with keymaps.MAC instead.

Both listening and typing need only Accessibility, the one permission the app
asks for. A listen-only tap would need Input Monitoring as well, so the tap is
a regular one (like Handy's) that passes every event through untouched.
"""

from __future__ import annotations

import queue
import threading
from typing import Callable

import ApplicationServices
import Quartz

from . import keymaps

_CAPS_LOCK = 57
_v_code = keymaps.MAC_V  # the key that types "v" in the user's layout; see prepare()


def prepare() -> None:
    """Find which key types "v", so Cmd+V pastes on Dvorak and other layouts.

    macOS only allows keyboard layout lookups on the main thread, so the app
    calls this once at startup. If the lookup fails, the QWERTY key is used.
    """
    global _v_code
    try:
        from pynput._util.darwin import get_unicode_to_keycode_map

        _v_code = get_unicode_to_keycode_map().get("v", keymaps.MAC_V)
    except Exception as exc:
        print(f"Couldn't read the keyboard layout ({exc}); pasting with the QWERTY V key.")


def permission_problem(prompt: bool = True) -> str | None:
    """None if we may listen and type; otherwise what the user has to allow.

    With prompt=True, macOS shows its own permission dialog the first time and
    adds Speechify to the list in System Settings.
    """
    options = {ApplicationServices.kAXTrustedCheckOptionPrompt: prompt}
    if ApplicationServices.AXIsProcessTrustedWithOptions(options):
        return None
    return (
        "Allow Speechify under System Settings > Privacy & Security > Accessibility, "
        "then restart it. (If you started it from a terminal, allow the terminal app instead.)"
    )


class Listener:
    """Calls on_press / on_release(name, vk, char) for every key, like pynput.

    The tap holds each key event until its callback returns, so the callback
    only queues the key and the handlers run on a thread of their own: opening
    the microphone can't make typing lag.
    """

    def __init__(self, on_press: Callable, on_release: Callable):
        self.on_press, self.on_release = on_press, on_release
        self._tap = None
        self._loop = None
        self._ready = threading.Event()
        self._events: queue.Queue = queue.Queue()
        self._thread = threading.Thread(target=self._run, daemon=True, name="quartz-keys")
        self._handler = threading.Thread(target=self._handle, daemon=True, name="quartz-keys-handler")

    def start(self) -> None:
        self._handler.start()
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
            Quartz.kCGEventTapOptionDefault, mask, self._callback, None,
        )
        if self._tap is None:  # no Accessibility permission
            self._ready.set()
            return
        source = Quartz.CFMachPortCreateRunLoopSource(None, self._tap, 0)
        self._loop = Quartz.CFRunLoopGetCurrent()
        Quartz.CFRunLoopAddSource(self._loop, source, Quartz.kCFRunLoopCommonModes)
        Quartz.CGEventTapEnable(self._tap, True)
        self._ready.set()
        Quartz.CFRunLoopRun()

    def _handle(self) -> None:
        while True:
            handler, name, char = self._events.get()
            try:
                handler(name, None, char)
            except Exception as exc:
                print(f"key handler failed: {exc}")

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
                    self._events.put((self.on_press, name, char))
            elif kind == Quartz.kCGEventKeyUp:
                self._events.put((self.on_release, name, char))
            elif kind == Quartz.kCGEventFlagsChanged:
                if code == _CAPS_LOCK:  # reported once per toggle, not per press
                    self._events.put((self.on_press, name, char))
                    self._events.put((self.on_release, name, char))
                elif code in keymaps.MAC_MODIFIER_BITS:
                    down = Quartz.CGEventGetFlags(event) & keymaps.MAC_MODIFIER_BITS[code]
                    self._events.put((self.on_press if down else self.on_release, name, char))
        except Exception as exc:  # an exception here would kill the event tap
            print(f"key tap failed: {exc}")
        return event


def _post(code: int, down: bool, flags: int) -> None:
    event = Quartz.CGEventCreateKeyboardEvent(None, code, down)
    Quartz.CGEventSetFlags(event, flags)
    Quartz.CGEventPost(Quartz.kCGHIDEventTap, event)


def cmd_v() -> None:
    cmd = Quartz.kCGEventFlagMaskCommand
    _post(keymaps.MAC_CMD, True, cmd)
    _post(_v_code, True, cmd)
    _post(_v_code, False, cmd)
    _post(keymaps.MAC_CMD, False, 0)


def shift_left(count: int) -> None:
    shift = Quartz.kCGEventFlagMaskShift
    for _ in range(count):
        _post(keymaps.MAC_LEFT, True, shift)
        _post(keymaps.MAC_LEFT, False, shift)
