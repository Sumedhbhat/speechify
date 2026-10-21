"""Native Windows key injection with SendInput.

pynput's Controller sends virtual keys without hardware scan codes. Some
apps (and the input stack on some Windows builds) ignore shortcuts sent that
way, so the paste shortcut is sent here as a real keyboard would: scan codes
plus virtual keys, all in one SendInput call so nothing can interleave.
"""

from __future__ import annotations

import ctypes
from ctypes import wintypes

INPUT_KEYBOARD = 1
KEYEVENTF_EXTENDEDKEY = 0x0001
KEYEVENTF_KEYUP = 0x0002
VK_CONTROL = 0x11
VK_SHIFT = 0x10
VK_LEFT = 0x25
VK_V = 0x56
_EXTENDED = {VK_LEFT, 0x26, 0x27, 0x28, 0x2D, 0x2E, 0x24, 0x23, 0x21, 0x22}

ULONG_PTR = ctypes.c_size_t


class _KEYBDINPUT(ctypes.Structure):
    _fields_ = [("wVk", wintypes.WORD), ("wScan", wintypes.WORD), ("dwFlags", wintypes.DWORD),
                ("time", wintypes.DWORD), ("dwExtraInfo", ULONG_PTR)]


class _MOUSEINPUT(ctypes.Structure):  # only here so the union has the right size
    _fields_ = [("dx", wintypes.LONG), ("dy", wintypes.LONG), ("mouseData", wintypes.DWORD),
                ("dwFlags", wintypes.DWORD), ("time", wintypes.DWORD), ("dwExtraInfo", ULONG_PTR)]


class _INPUTUNION(ctypes.Union):
    _fields_ = [("ki", _KEYBDINPUT), ("mi", _MOUSEINPUT)]


class _INPUT(ctypes.Structure):
    _fields_ = [("type", wintypes.DWORD), ("u", _INPUTUNION)]


_user32 = ctypes.WinDLL("user32", use_last_error=True)
_user32.SendInput.argtypes = (wintypes.UINT, ctypes.POINTER(_INPUT), ctypes.c_int)
_user32.SendInput.restype = wintypes.UINT


def _event(vk: int, up: bool) -> _INPUT:
    scan = _user32.MapVirtualKeyW(vk, 0)  # MAPVK_VK_TO_VSC
    flags = (KEYEVENTF_KEYUP if up else 0) | (KEYEVENTF_EXTENDEDKEY if vk in _EXTENDED else 0)
    return _INPUT(type=INPUT_KEYBOARD, u=_INPUTUNION(ki=_KEYBDINPUT(vk, scan, flags, 0, 0)))


def send(events: list[tuple[int, bool]]) -> None:
    """Send (vk, is_key_up) events atomically. Raises if Windows rejected them."""
    inputs = (_INPUT * len(events))(*(_event(vk, up) for vk, up in events))
    sent = _user32.SendInput(len(events), inputs, ctypes.sizeof(_INPUT))
    if sent != len(events):
        raise OSError(ctypes.get_last_error(), f"SendInput sent {sent} of {len(events)} key events")


def chord(*vks: int) -> None:
    """Press keys in order, release in reverse, like a person doing Ctrl+V."""
    send([(vk, False) for vk in vks] + [(vk, True) for vk in reversed(vks)])


def ctrl_v() -> None:
    chord(VK_CONTROL, VK_V)


def shift_left(times: int) -> None:
    send([(VK_SHIFT, False)] + [(VK_LEFT, up) for _ in range(times) for up in (False, True)]
         + [(VK_SHIFT, True)])
