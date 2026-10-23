"""Is the computer plugged in? Background learning only runs on mains power.

No extra packages: Windows asks the kernel through ctypes, Linux reads
/sys/class/power_supply, and macOS asks `pmset`. A machine with no battery
(a desktop) counts as plugged in. When the answer can't be read at all,
it's "not plugged in", so a laptop never drains its battery training.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path


def on_mains_power() -> bool:
    try:
        if sys.platform == "win32":
            return _windows()
        if sys.platform == "darwin":
            return _macos(subprocess.run(["pmset", "-g", "batt"], capture_output=True, text=True,
                                         timeout=5).stdout)
        return _linux(Path("/sys/class/power_supply"))
    except Exception as exc:
        print(f"power: couldn't read the power state ({exc}); assuming battery")
        return False


def _windows() -> bool:
    import ctypes
    from ctypes import wintypes

    class SYSTEM_POWER_STATUS(ctypes.Structure):
        _fields_ = [
            ("ACLineStatus", wintypes.BYTE),
            ("BatteryFlag", wintypes.BYTE),
            ("BatteryLifePercent", wintypes.BYTE),
            ("SystemStatusFlag", wintypes.BYTE),
            ("BatteryLifeTime", wintypes.DWORD),
            ("BatteryFullLifeTime", wintypes.DWORD),
        ]

    status = SYSTEM_POWER_STATUS()
    if not ctypes.windll.kernel32.GetSystemPowerStatus(ctypes.byref(status)):
        raise OSError("GetSystemPowerStatus failed")
    return windows_status(status.ACLineStatus & 0xFF, status.BatteryFlag & 0xFF)


def windows_status(ac_line: int, battery_flag: int) -> bool:
    """ACLineStatus: 0 offline, 1 online, 255 unknown. BatteryFlag 128 = no battery."""
    if ac_line == 1:
        return True
    if ac_line == 255:
        return battery_flag != 255 and bool(battery_flag & 128)
    return False


def _linux(root: Path) -> bool:
    mains_online = None
    has_battery = False
    for supply in sorted(root.glob("*")) if root.exists() else []:
        kind = _read(supply / "type")
        if kind == "Mains" or (kind == "USB" and supply.name.lower().startswith(("ac", "adp"))):
            online = _read(supply / "online")
            if online is not None:
                mains_online = bool(mains_online) or online == "1"
        elif kind == "Battery" and _read(supply / "scope") != "Device":  # not a mouse or headset
            has_battery = True
            status = _read(supply / "status")
            if status in ("Charging", "Full") and mains_online is None:
                mains_online = True
    if mains_online is not None:
        return mains_online
    return not has_battery  # desktops and most VMs have neither


def _read(path: Path) -> str | None:
    try:
        return path.read_text().strip()
    except OSError:
        return None


def _macos(pmset_output: str) -> bool:
    first = pmset_output.splitlines()[0] if pmset_output else ""
    if "AC Power" in first:
        return True
    if "Battery Power" in first or "UPS Power" in first:
        return False
    raise OSError(f"unexpected pmset output: {first!r}")
