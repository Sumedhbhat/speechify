#!/usr/bin/env python3
"""Build the native macOS desktop-boundary fixture for the coverage runner."""
import os
from pathlib import Path
import subprocess

root = Path(__file__).resolve().parents[1]
destination = root / ".build/macos-desktop.dylib"
destination.parent.mkdir(parents=True, exist_ok=True)
subprocess.run([os.environ.get("CC", "cc"), "-std=c11", "-dynamiclib", "-Wall", "-Wextra", "-Werror",
                str(root / "tests/fixtures/macos-desktop.m"), "-framework", "ApplicationServices",
                "-framework", "CoreFoundation", "-framework", "AppKit", "-o", str(destination)], check=True)
print(destination)
