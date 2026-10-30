#!/usr/bin/env python3
"""Build the synthetic ALSA driver used by capture behavior tests."""
import os
from pathlib import Path
import shlex
import subprocess

root = Path(__file__).resolve().parents[1]
destination = root / ".build/alsa"
destination.mkdir(parents=True, exist_ok=True)
flags = shlex.split(subprocess.check_output(["pkg-config", "--cflags", "--libs", "alsa"], text=True))
subprocess.run([os.environ.get("CC", "cc"), "-shared", "-fPIC", "-DPIC", "-Wall", "-Wextra", "-Werror",
                str(root / "tests/fixtures/alsa.c"), "-o", str(destination / "libasound_module_pcm_speechify_fixture.so"), *flags], check=True)
print(destination)
