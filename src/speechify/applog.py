"""Where the installed app's output goes, and opening files for the user.

Started from the Start menu, the app menu, Finder or the Dock, the installed
app has no console, so what it prints goes to a log file.
"""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path


def log_path() -> Path:
    if sys.platform == "win32":
        base = Path(os.environ.get("LOCALAPPDATA") or Path.home() / "AppData" / "Local")
        return base / "Speechify" / "speechify.log"
    if sys.platform == "darwin":
        return Path.home() / "Library" / "Logs" / "Speechify.log"
    state = Path(os.environ.get("XDG_STATE_HOME") or Path.home() / ".local" / "state")
    return state / "speechify" / "speechify.log"


def has_terminal() -> bool:
    return sys.stdout is not None and hasattr(sys.stdout, "isatty") and sys.stdout.isatty()


def redirect_to_file() -> None:
    """Send stdout and stderr to the log file when the frozen app has no
    terminal. Running from source, or from a terminal, leaves them alone."""
    if not getattr(sys, "frozen", False) or has_terminal():
        return
    log = log_path()
    log.parent.mkdir(parents=True, exist_ok=True)
    _trim(log)
    stream = open(log, "a", buffering=1, encoding="utf-8", errors="replace")
    # At the descriptor level too, so the model worker's output lands there.
    os.dup2(stream.fileno(), 1)
    os.dup2(stream.fileno(), 2)
    if sys.platform == "win32":
        # A windowed process has no standard handles, and child processes
        # inherit those, not the C runtime's descriptors.
        import ctypes
        import msvcrt

        kernel32 = ctypes.windll.kernel32
        kernel32.SetStdHandle(-11, msvcrt.get_osfhandle(1))  # STD_OUTPUT_HANDLE
        kernel32.SetStdHandle(-12, msvcrt.get_osfhandle(2))  # STD_ERROR_HANDLE
    sys.stdout = sys.stderr = stream


def _trim(log: Path, keep: int = 2_000_000) -> None:
    """Keep the log from growing forever: past 4 MB, keep the last 2 MB."""
    try:
        if log.stat().st_size > 2 * keep:
            with open(log, "rb") as f:
                f.seek(-keep, os.SEEK_END)
                tail = f.read()
            log.write_bytes(tail[tail.find(b"\n") + 1:])
    except OSError:
        pass


def open_in_os(path: Path) -> None:
    """Open a file or folder in the system's default app."""
    if sys.platform == "win32":
        os.startfile(path)  # type: ignore[attr-defined]
    elif sys.platform == "darwin":
        subprocess.Popen(["open", str(path)])
    else:
        subprocess.Popen(["xdg-open", str(path)])
