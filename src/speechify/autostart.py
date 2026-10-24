"""Start Speechify when you sign in, on each platform.

    Windows  a value under HKCU\\...\\Run (the installer's "Start Speechify when I
             sign in" box writes the same one)
    macOS    a LaunchAgent in ~/Library/LaunchAgents
    Linux    an XDG autostart entry in ~/.config/autostart
"""

from __future__ import annotations

import os
import plistlib
import shlex
import sys
from pathlib import Path

NAME = "Speechify"
LABEL = "io.github.sumedhbhat.speechify"
RUN_KEY = r"Software\Microsoft\Windows\CurrentVersion\Run"


def command() -> list[str]:
    """What starts this copy of Speechify."""
    if os.environ.get("APPIMAGE"):  # the AppImage's own path, not its mount point
        return [os.environ["APPIMAGE"]]
    if getattr(sys, "frozen", False):
        return [sys.executable]
    return [sys.executable, "-m", "speechify"]


def _launch_agent() -> Path:
    return Path.home() / "Library" / "LaunchAgents" / f"{LABEL}.plist"


def _desktop_entry() -> Path:
    config = Path(os.environ.get("XDG_CONFIG_HOME") or Path.home() / ".config")
    return config / "autostart" / "speechify.desktop"


def is_enabled() -> bool:
    if sys.platform == "win32":
        import winreg

        try:
            with winreg.OpenKey(winreg.HKEY_CURRENT_USER, RUN_KEY) as key:
                winreg.QueryValueEx(key, NAME)
            return True
        except OSError:
            return False
    if sys.platform == "darwin":
        return _launch_agent().exists()
    return _desktop_entry().exists()


def set_enabled(enabled: bool) -> None:
    if sys.platform == "win32":
        import subprocess
        import winreg

        with winreg.CreateKey(winreg.HKEY_CURRENT_USER, RUN_KEY) as key:
            if enabled:
                winreg.SetValueEx(key, NAME, 0, winreg.REG_SZ, subprocess.list2cmdline(command()))
            else:
                try:
                    winreg.DeleteValue(key, NAME)
                except FileNotFoundError:
                    pass
        return
    path = _launch_agent() if sys.platform == "darwin" else _desktop_entry()
    if not enabled:
        path.unlink(missing_ok=True)
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    if sys.platform == "darwin":
        path.write_bytes(plistlib.dumps({
            "Label": LABEL,
            "ProgramArguments": command(),
            "RunAtLoad": True,
            "ProcessType": "Interactive",
        }))
    else:
        path.write_text(
            "[Desktop Entry]\n"
            "Type=Application\n"
            f"Name={NAME}\n"
            "Comment=Hold a key, talk, and the text is pasted\n"
            f"Exec={shlex.join(command())}\n"
            "Icon=speechify\n"
            "Terminal=false\n"
            "X-GNOME-Autostart-enabled=true\n",
            encoding="utf-8",
        )
