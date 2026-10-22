"""How much GPU memory there is, without loading CUDA into the app.

Asks `nvidia-smi`, which comes with the NVIDIA driver on Windows and Linux,
so the app process stays free of torch.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
from dataclasses import dataclass


@dataclass(frozen=True)
class GpuInfo:
    name: str
    total_gb: float
    free_gb: float


def query() -> GpuInfo | None:
    """The first NVIDIA GPU, or None if there isn't one (or no driver)."""
    exe = shutil.which("nvidia-smi")
    if exe is None and sys.platform == "win32":
        candidate = r"C:\Windows\System32\nvidia-smi.exe"
        exe = candidate if os.path.exists(candidate) else None
    if exe is None:
        return None
    try:
        out = subprocess.run(
            [exe, "--query-gpu=name,memory.total,memory.free", "--format=csv,noheader,nounits"],
            capture_output=True, text=True, timeout=10,
            creationflags=subprocess.CREATE_NO_WINDOW if sys.platform == "win32" else 0,
        ).stdout
    except (OSError, subprocess.SubprocessError):
        return None
    return parse(out)


def parse(nvidia_smi_csv: str) -> GpuInfo | None:
    for line in nvidia_smi_csv.splitlines():
        parts = [p.strip() for p in line.split(",")]
        if len(parts) == 3:
            try:
                return GpuInfo(parts[0], float(parts[1]) / 1024, float(parts[2]) / 1024)
            except ValueError:
                continue
    return None
