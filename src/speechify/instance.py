"""One Speechify at a time.

The first copy listens on a localhost port derived from the user and data
folder. Starting Speechify again (from the Start menu, the app menu or the
Dock) connects to it, asks it to open its settings, and exits, the way Handy
and most tray apps behave. Two copies would fight over the talk key.
"""

from __future__ import annotations

import getpass
import socket
import sys
import threading
import zlib
from typing import Callable

from .config import data_dir

HELLO = b"speechify\n"


def port() -> int:
    who = f"{getpass.getuser()}:{data_dir()}".encode()
    return 49152 + zlib.crc32(who) % 16000


class Instance:
    def __init__(self):
        self.on_show: Callable[[], None] | None = None
        self._server: socket.socket | None = None

    def claim(self) -> bool:
        """True if this is the only Speechify. False if another one is
        running; it has been asked to show its settings."""
        server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        if sys.platform == "win32":
            server.setsockopt(socket.SOL_SOCKET, socket.SO_EXCLUSIVEADDRUSE, 1)
        else:  # still refuses a second listener; only skips TIME_WAIT
            server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        try:
            server.bind(("127.0.0.1", port()))
        except OSError:
            server.close()
            if _ask_to_show():
                return False
            print(f"Port {port()} is taken by another program; not checking for a second Speechify.")
            return True
        server.listen(4)
        self._server = server
        threading.Thread(target=self._serve, daemon=True, name="instance").start()
        return True

    def _serve(self) -> None:
        while self._server is not None:
            try:
                conn, _ = self._server.accept()
            except OSError:
                return
            with conn:
                conn.settimeout(2)
                try:
                    if conn.recv(64).startswith(b"show"):
                        conn.sendall(HELLO)
                        if self.on_show:
                            self.on_show()
                    conn.recv(1)  # let the client close first, so no TIME_WAIT here
                except OSError:
                    pass

    def close(self) -> None:
        server, self._server = self._server, None
        if server is not None:
            server.close()


def _ask_to_show() -> bool:
    try:
        with socket.create_connection(("127.0.0.1", port()), timeout=2) as conn:
            conn.sendall(b"show\n")
            return conn.recv(64) == HELLO
    except OSError:
        return False
