#!/usr/bin/env python3
"""Build the X11 clipboard boundary fixture for isolated CLI tests."""
import os
from pathlib import Path
import subprocess

root = Path(__file__).resolve().parents[1]
destination = root / ".build/x11-desktop.so"
destination.parent.mkdir(parents=True, exist_ok=True)
subprocess.run([os.environ.get("CC", "cc"), "-shared", "-fPIC", "-Wall", "-Wextra", "-Werror",
                str(root / "tests/fixtures/x11-desktop.c"), "-o", str(destination),
                "-Wl,--no-as-needed", "-lX11", "-ldl"], check=True)
print(destination)
