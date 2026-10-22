"""Keyboard input and output through the Linux kernel (evdev and uinput).

Wayland doesn't let one app see another app's keys, or type into it, so
pynput's X11 backend only works for X11 windows there. Reading /dev/input
and typing through a /dev/uinput virtual keyboard works under every
compositor (and X11), because it sits below them. It needs the user in the
`input` group, which scripts/install.sh sets up.

Standard library only: python-evdev would need a C compiler to install.
"""

from __future__ import annotations

import errno
import fcntl
import os
import selectors
import struct
import threading
import time
from pathlib import Path
from typing import Callable

from . import keymaps

EV_SYN, EV_KEY = 0, 1
_EVENT = struct.Struct("llHHi")  # struct input_event: timeval, type, code, value
VIRTUAL_NAME = "speechify virtual keyboard"
RESCAN_SECONDS = 2.0  # how often to look for keyboards plugged in later

PERMISSION_HELP = (
    "Speechify can't read the keyboard. On Wayland it reads keys through /dev/input, "
    "which needs your user in the 'input' group:\n"
    "    sudo usermod -aG input $USER\n"
    "then log out and back in. scripts/install.sh can do this for you."
)
UINPUT_HELP = (
    "Speechify can't type into other apps: /dev/uinput isn't writable. Run scripts/install.sh "
    "again, or allow the input group to use it:\n"
    '    echo \'KERNEL=="uinput", GROUP="input", MODE="0660", OPTIONS+="static_node=uinput"\' '
    "| sudo tee /etc/udev/rules.d/60-speechify-uinput.rules\n"
    "    sudo modprobe uinput && sudo udevadm control --reload && sudo udevadm trigger /dev/uinput"
)


def _ioc(direction: int, kind: str, nr: int, size: int) -> int:
    return (direction << 30) | (size << 16) | (ord(kind) << 8) | nr


def _eviocgname(size: int) -> int:
    return _ioc(2, "E", 0x06, size)


def _eviocgbit(ev: int, size: int) -> int:
    return _ioc(2, "E", 0x20 + ev, size)


def _device_name(fd: int) -> str:
    buf = bytearray(256)
    try:
        fcntl.ioctl(fd, _eviocgname(len(buf)), buf)
    except OSError:
        return ""
    return buf.split(b"\0", 1)[0].decode(errors="replace")


def _has_keyboard_keys(fd: int) -> bool:
    """True for devices with ordinary keys (not mice, which only have buttons)."""
    bits = bytearray(96)  # KEY_MAX 0x2ff -> 768 bits
    try:
        fcntl.ioctl(fd, _eviocgbit(EV_KEY, len(bits)), bits)
    except OSError:
        return False
    return any(bits[1:31])  # key codes 8-247: letters, modifiers, F-keys


def decode(data: bytes) -> list[tuple[int, int]]:
    """Raw event bytes -> [(key code, value)] for key events. 1 = down, 0 = up, 2 = repeat."""
    out = []
    for offset in range(0, len(data) - _EVENT.size + 1, _EVENT.size):
        _, _, kind, code, value = _EVENT.unpack_from(data, offset)
        if kind == EV_KEY:
            out.append((code, value))
    return out


class Listener:
    """Calls on_press / on_release(name, vk, char) for every keyboard, like pynput."""

    def __init__(self, on_press: Callable, on_release: Callable, ignore: str = VIRTUAL_NAME):
        self.on_press, self.on_release, self.ignore = on_press, on_release, ignore
        self._selector = selectors.DefaultSelector()
        self._open: dict[str, int] = {}
        # Devices that aren't keyboards (mice, the lid switch), by path and
        # inode, so the rescan doesn't reopen them; a replugged device gets a
        # new inode and is checked again.
        self._rejected: set[tuple[str, int]] = set()
        self._wake_r, self._wake_w = os.pipe()
        self._selector.register(self._wake_r, selectors.EVENT_READ, None)
        self._stopped = False
        self._thread = threading.Thread(target=self._run, daemon=True, name="evdev-keys")

    def start(self) -> None:
        denied = self._scan()
        if not self._open:
            if denied:
                raise PermissionError(PERMISSION_HELP)
            print("No keyboard found under /dev/input yet; waiting for one.")
        self._thread.start()

    def stop(self) -> None:
        if self._stopped:
            return
        # Wake first, then flag: the thread closes the pipe once it sees the flag.
        os.write(self._wake_w, b"x")
        self._stopped = True

    def _scan(self) -> bool:
        """Open keyboards we haven't seen. Returns True if any device was off limits."""
        denied = False
        for path in sorted(Path("/dev/input").glob("event*")):
            if str(path) in self._open:
                continue
            try:
                node = (str(path), path.stat().st_ino)
            except OSError:
                continue
            if node in self._rejected:
                continue
            try:
                fd = os.open(path, os.O_RDONLY | os.O_NONBLOCK)
            except PermissionError:
                denied = True
                continue
            except OSError:
                continue
            if not _has_keyboard_keys(fd) or _device_name(fd) == self.ignore:
                os.close(fd)
                self._rejected.add(node)
                continue
            self._open[str(path)] = fd
            self._selector.register(fd, selectors.EVENT_READ, str(path))
        return denied

    def _close(self, path: str) -> None:
        fd = self._open.pop(path)
        self._selector.unregister(fd)
        os.close(fd)

    def _run(self) -> None:
        last_scan = time.monotonic()
        while not self._stopped:
            for key, _ in self._selector.select(timeout=RESCAN_SECONDS):
                if key.data is None:
                    continue  # woken by stop()
                try:
                    data = os.read(key.fd, _EVENT.size * 64)
                except BlockingIOError:
                    continue
                except OSError as exc:  # unplugged
                    if exc.errno in (errno.ENODEV, errno.EIO, errno.EBADF):
                        self._close(key.data)
                    continue
                for code, value in decode(data):
                    if value == 2:
                        continue  # auto-repeat: callers only care about down and up
                    name, char = keymaps.EVDEV.get(code, (None, None))
                    if name is None and char is None:
                        continue
                    (self.on_press if value else self.on_release)(name, None, char)
            if time.monotonic() - last_scan >= RESCAN_SECONDS:
                self._scan()
                last_scan = time.monotonic()
        for path in list(self._open):
            self._close(path)
        self._selector.close()
        os.close(self._wake_r)
        os.close(self._wake_w)


class Keyboard:
    """A virtual keyboard that types into whatever window has focus."""

    def __init__(self, name: str = VIRTUAL_NAME):
        try:
            self._fd = os.open("/dev/uinput", os.O_WRONLY | os.O_NONBLOCK)
        except OSError as exc:
            raise PermissionError(UINPUT_HELP) from exc
        try:
            fcntl.ioctl(self._fd, _ioc(1, "U", 100, 4), EV_KEY)  # UI_SET_EVBIT
            for code in range(1, 249):  # every ordinary key, so it's classed as a keyboard
                fcntl.ioctl(self._fd, _ioc(1, "U", 101, 4), code)  # UI_SET_KEYBIT
            # struct uinput_setup: input_id (bus, vendor, product, version), name[80], ff_effects_max
            setup = struct.pack("HHHH80sI", 0x06, 0x5350, 0x0001, 1, name.encode()[:79], 0)
            fcntl.ioctl(self._fd, _ioc(1, "U", 3, len(setup)), setup)  # UI_DEV_SETUP (Linux 4.5+)
            fcntl.ioctl(self._fd, _ioc(0, "U", 1, 0))  # UI_DEV_CREATE
        except OSError as exc:
            os.close(self._fd)
            raise OSError(exc.errno, f"Couldn't create a virtual keyboard through /dev/uinput: {exc}") from exc
        # The compositor needs a moment to pick up a new device before it
        # delivers its keys; this runs once, at startup.
        time.sleep(0.3)
        self._lock = threading.Lock()

    def _emit(self, code: int, value: int) -> None:
        os.write(self._fd, _EVENT.pack(0, 0, EV_KEY, code, value) + _EVENT.pack(0, 0, EV_SYN, 0, 0))

    def chord(self, *codes: int) -> None:
        """Press the keys in order, then release them in reverse."""
        with self._lock:
            for code in codes:
                self._emit(code, 1)
            time.sleep(0.01)
            for code in reversed(codes):
                self._emit(code, 0)

    def close(self) -> None:
        fcntl.ioctl(self._fd, _ioc(0, "U", 2, 0))  # UI_DEV_DESTROY
        os.close(self._fd)


_keyboard: Keyboard | None = None


def keyboard() -> Keyboard:
    global _keyboard
    if _keyboard is None:
        _keyboard = Keyboard()
    return _keyboard


def ctrl_v() -> None:
    keyboard().chord(keymaps.EV_LEFTCTRL, keymaps.EV_V)


def shift_left(count: int) -> None:
    kb = keyboard()
    with kb._lock:
        kb._emit(keymaps.EV_LEFTSHIFT, 1)
        for i in range(count):
            kb._emit(keymaps.EV_LEFT, 1)
            kb._emit(keymaps.EV_LEFT, 0)
            if i % 16 == 15:
                # A long burst overflows the compositor's event buffer and
                # drops presses, which would select too little text.
                time.sleep(0.005)
        kb._emit(keymaps.EV_LEFTSHIFT, 0)
