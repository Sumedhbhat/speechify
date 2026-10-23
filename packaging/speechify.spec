# PyInstaller spec for the installed app. packaging/build.sh runs it; see that
# file for what goes around it on each platform.
#
# The app freezes into one folder with its own Python, plus uv, which builds
# each speech model's environment on first use (src/speechify/engines/host.py).

import os
import shutil
import sys
from pathlib import Path

from PyInstaller.utils.hooks import collect_submodules

root = Path(SPECPATH).parent
src = root / "src" / "speechify"
build = root / "packaging" / "build"
version = (src / "VERSION").read_text().strip()

uv = os.environ.get("SPEECHIFY_UV") or shutil.which("uv")
if not uv:
    sys.exit("uv not found; set SPEECHIFY_UV to the uv binary to bundle")

datas = [
    (str(src / "VERSION"), "speechify"),
    (str(src / "selftest_data"), "speechify/selftest_data"),
]
# Copied out to each model environment at runtime, so they're needed as files.
datas += [(str(src / "engines" / f), "speechify/engines") for f in ("__init__.py", "backends.py", "worker.py")]
# Automatic retraining runs scripts/finetune.py in its own environment, with
# copies of these (src/speechify/students.py stages them).
datas += [(str(src / f), "speechify") for f in ("__init__.py", "audio.py", "config.py", "labels.py", "metrics.py")]
datas += [(str(root / "scripts" / "finetune.py"), "speechify/_scripts")]
binaries = [(uv, ".")]
runtime_hooks = []
if sys.platform.startswith("linux"):
    binaries.append((str(build / "libportaudio.so.2"), "."))
    runtime_hooks.append(str(root / "packaging" / "rthook_portaudio.py"))

# Keyboard, window and platform modules are imported lazily, and pynput picks
# its backend at runtime.
hiddenimports = collect_submodules("speechify", on_error="ignore") + collect_submodules("pynput", on_error="ignore")

a = Analysis(
    [str(root / "packaging" / "entry.py")],
    pathex=[str(root / "src")],
    binaries=binaries,
    datas=datas,
    hiddenimports=hiddenimports,
    runtime_hooks=runtime_hooks,
    # pynput's uinput backend; Speechify has its own (linuxkeys.py).
    excludes=["evdev", "pytest", "PIL"],
)
pyz = PYZ(a.pure)

name = "speechify" if sys.platform.startswith("linux") else "Speechify"
icon = {"win32": "icon.ico", "darwin": "icon.icns"}.get(sys.platform)
exe = EXE(
    pyz,
    a.scripts,
    [],
    exclude_binaries=True,
    name=name,
    # Windows keeps a console (started minimized) for the per-dictation log and
    # so closing it quits; the Mac app logs to a file instead.
    console=sys.platform != "darwin",
    icon=str(build / icon) if icon else None,
    version=str(build / "version_info.txt") if sys.platform == "win32" else None,
)
coll = COLLECT(exe, a.binaries, a.datas, name=name)

if sys.platform == "darwin":
    short_version = version.split("-")[0]
    app = BUNDLE(
        coll,
        name="Speechify.app",
        icon=str(build / "icon.icns"),
        bundle_identifier="io.github.sumedhbhat.speechify",
        version=short_version,
        info_plist={
            "CFBundleName": "Speechify",
            "CFBundleDisplayName": "Speechify",
            "CFBundleShortVersionString": short_version,
            "CFBundleVersion": version,
            "LSUIElement": True,  # no Dock icon; it lives in the background
            "LSMinimumSystemVersion": "11.0",
            "NSMicrophoneUsageDescription": "Speechify records while you hold the talk key and transcribes it on this Mac.",
        },
    )
