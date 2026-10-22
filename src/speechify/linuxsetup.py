"""One-time Linux setup that the installed app does for itself.

- On Wayland, Speechify reads keys from /dev/input and types through
  /dev/uinput (see linuxkeys.py). That needs the user in the `input` group
  and a udev rule; offer_wayland_access() asks, then sets both up through
  pkexec, the desktop's own password prompt.
- Pasting needs wl-clipboard (Wayland) or xclip / xsel (X11).
  clipboard_problem() names the package to install.
- Started from an AppImage, register_appimage() adds Speechify to the app
  menu, pointing at wherever the AppImage is now.
"""

from __future__ import annotations

import os
import shlex
import shutil
import subprocess
import sys
from pathlib import Path

UINPUT_RULE = 'KERNEL=="uinput", SUBSYSTEM=="misc", GROUP="input", MODE="0660", OPTIONS+="static_node=uinput"'

# Run as root by pkexec, with the user's name as $1.
_ACCESS_SCRIPT = f"""set -e
usermod -aG input "$1"
echo '{UINPUT_RULE}' > /etc/udev/rules.d/60-speechify-uinput.rules
echo uinput > /etc/modules-load.d/speechify-uinput.conf
modprobe uinput || true
udevadm control --reload-rules
udevadm trigger --subsystem-match=misc --sysname-match=uinput || true
"""


def _is_wayland() -> bool:
    return bool(os.environ.get("WAYLAND_DISPLAY")) or os.environ.get("XDG_SESSION_TYPE") == "wayland"


def _install_command(package: str) -> str:
    for pm, cmd in (("apt-get", "sudo apt install"), ("dnf", "sudo dnf install"),
                    ("pacman", "sudo pacman -S"), ("zypper", "sudo zypper install")):
        if shutil.which(pm):
            return f"{cmd} {package}"
    return f"install {package} with your package manager"


def clipboard_problem() -> str | None:
    """What to install so pyperclip can reach the clipboard, if anything."""
    if _is_wayland():
        if shutil.which("wl-copy") and shutil.which("wl-paste"):
            return None
        package = "wl-clipboard"
    else:
        if shutil.which("xclip") or shutil.which("xsel"):
            return None
        package = "xclip"
    return f"Speechify pastes through the clipboard, which needs {package}:\n    {_install_command(package)}"


def _restart_command() -> list[str]:
    if os.environ.get("APPIMAGE"):
        return [os.environ["APPIMAGE"], *sys.argv[1:]]
    if getattr(sys, "frozen", False):
        return [sys.executable, *sys.argv[1:]]
    return [sys.executable, "-m", "speechify", *sys.argv[1:]]


def offer_wayland_access(problem: str) -> None:
    """Ask to set up /dev/input and /dev/uinput access. If that works,
    restarts Speechify with the new group right away (no need to log out),
    so this only returns if the user said no or the setup failed."""
    from tkinter import messagebox

    if not shutil.which("pkexec"):
        return
    if not messagebox.askyesno(
        "Speechify",
        "On Wayland, Speechify reads the talk key from /dev/input and pastes through a "
        "virtual keyboard. That needs your user in the 'input' group, which lets programs "
        "you run read keystrokes, the same access every app has on X11.\n\n"
        "Set it up now? You'll be asked for your password.",
    ):
        return
    user = os.environ.get("USER") or subprocess.run(["id", "-un"], capture_output=True, text=True).stdout.strip()
    done = subprocess.run(["pkexec", "/bin/sh", "-c", _ACCESS_SCRIPT, "sh", user])
    if done.returncode != 0:
        messagebox.showerror("Speechify", f"The setup didn't finish.\n\n{problem}")
        return
    print("Set up keyboard access; restarting Speechify with the input group.")
    # The new group only applies to new logins; sg gives this one process it now.
    cmd = shlex.join(_restart_command())
    if shutil.which("sg"):
        os.execvp("sg", ["sg", "input", "-c", cmd])
    messagebox.showinfo("Speechify", "Done. Log out and back in once, then start Speechify again.")
    raise SystemExit(0)


def _write_if_changed(path: Path, data: bytes) -> None:
    if path.exists() and path.read_bytes() == data:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)


def _desktop_quote(path: str) -> str:
    # Desktop entries quote with double quotes only, and unescape twice.
    escaped = "".join("\\" + c if c in '"`$\\' else c for c in path)
    return '"' + escaped.replace("\\", "\\\\") + '"'


def register_appimage() -> None:
    """Add (or update) the app menu entry for the AppImage we were started from."""
    appimage, appdir = os.environ.get("APPIMAGE"), os.environ.get("APPDIR")
    if not appimage or not appdir:
        return
    share = Path(os.environ.get("XDG_DATA_HOME") or Path.home() / ".local" / "share")
    try:
        icon = Path(appdir) / "speechify.png"
        if icon.exists():
            _write_if_changed(share / "icons" / "hicolor" / "256x256" / "apps" / "speechify.png", icon.read_bytes())
        entry = (
            "[Desktop Entry]\n"
            "Type=Application\n"
            "Name=Speechify\n"
            "Comment=Hold a key, talk, and the text is pasted\n"
            f"Exec={_desktop_quote(appimage)}\n"
            "Icon=speechify\n"
            "Terminal=false\n"
            "Categories=Utility;Accessibility;\n"
        )
        _write_if_changed(share / "applications" / "speechify.desktop", entry.encode())
    except OSError as exc:
        print(f"Couldn't add Speechify to the app menu: {exc}")
