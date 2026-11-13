#!/usr/bin/env python3
"""Download the pinned transcribe.cpp runtime for this machine.

The speech engine is a prebuilt native library (CPU, Vulkan and Metal
backends included), published as a Python wheel. This script fetches the
wheel for the current platform, checks its SHA-256 and unpacks the
libraries into src-tauri/resources/native, where the app bundles them,
together with the pinned voice-detection model.
It installs nothing and needs only the Python standard library.
"""

import hashlib
import io
import json
import pathlib
import platform
import sys
import urllib.request
import zipfile

VERSION = "0.4.1"
BASE = "https://files.pythonhosted.org/packages"
# (system, machine) -> (wheel path under BASE, sha256)
WHEELS = {
    ("Linux", "x86_64"): (
        "89/90/745a4c6bf068a4aec400e32f81f6607475ca4dde8810f6c5c91534201cfa/transcribe_cpp_native-0.4.1-py3-none-manylinux_2_27_x86_64.manylinux_2_28_x86_64.whl",
        "80011910fdc45adfeb78bf1a0e8813e9ad8be54b4f135b20e815388a11e78a50",
    ),
    ("Linux", "aarch64"): (
        "85/26/f70a75ba3a7d6f950dc08e14a877bc811a4b54035d9e847b9ca629d09cde/transcribe_cpp_native-0.4.1-py3-none-manylinux_2_27_aarch64.manylinux_2_28_aarch64.whl",
        "371e0fd638436123580401284c61610df847d094f5893f38339a6cccd1a16e51",
    ),
    ("Darwin", "arm64"): (
        "ac/bd/92e9e74cf4a59e118fd6b5c65c8f4f3b3c81b7c8030707a6a178764eea3c/transcribe_cpp_native-0.4.1-py3-none-macosx_11_0_arm64.whl",
        "59fa4672e79cb5282b5d0c435506a22cd47bc6390a6bfee354ba46cae929a7c1",
    ),
    ("Darwin", "x86_64"): (
        "bd/6b/98d884a92c57b722f8f720e996775425509a82db418a4157a941018f8d67/transcribe_cpp_native-0.4.1-py3-none-macosx_11_0_x86_64.whl",
        "06d8b7415724a96704630f2056f07d8dd7da4504adaa26edae2de9c97b261d72",
    ),
    ("Windows", "amd64"): (
        "b3/d9/41cbb7195f0fbcf3fea8675da8e597b3b2fc5b553a754120823c62d2f54a/transcribe_cpp_native-0.4.1-py3-none-win_amd64.whl",
        "ed0df2783c70c08eca406cbab5b79e2f0da3cac575cbc562b402b953fff508df",
    ),
}


# Silero VAD 6.2 (MIT), converted for transcribe.cpp: finds speech in a
# recording, including quiet and whispered speech. About 1 MB.
DETECTOR = (
    "silero-vad-v6.2-F32.gguf",
    "https://huggingface.co/handy-computer/silero-vad-v6.2-gguf/resolve/"
    "a9b2d61c20ae4ddf3644edb34969e8185a485448/silero-vad-v6.2-F32.gguf",
    "0d871a800c420fdd5d1b17a3aea4296d1c39608f0ff4a9295eb76457eb1c88e6",
)


def fetch_detector(dest: pathlib.Path) -> bool:
    name, url, digest = DETECTOR
    target = dest / name
    if target.exists() and hashlib.sha256(target.read_bytes()).hexdigest() == digest:
        return True
    print("Downloading the voice detector")
    data = urllib.request.urlopen(url, timeout=120).read()
    if hashlib.sha256(data).hexdigest() != digest:
        print("Checksum mismatch: refusing to save the voice detector", file=sys.stderr)
        return False
    dest.mkdir(parents=True, exist_ok=True)
    target.write_bytes(data)
    return True


def main() -> int:
    root = pathlib.Path(__file__).resolve().parents[1]
    dest = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else root / "src-tauri" / "resources" / "native"
    key = (platform.system(), platform.machine().lower())
    if key not in WHEELS:
        print(f"No prebuilt speech runtime for {key[0]} {key[1]}", file=sys.stderr)
        return 1
    path, digest = WHEELS[key]
    marker = dest / "provenance.json"
    if marker.exists() and json.loads(marker.read_text()).get("sha256") == digest:
        print(f"Speech runtime {VERSION} already in {dest}")
        return 0 if fetch_detector(dest) else 1
    url = f"{BASE}/{path}"
    print(f"Downloading speech runtime {VERSION} for {key[0]} {key[1]}")
    data = urllib.request.urlopen(url, timeout=120).read()
    if hashlib.sha256(data).hexdigest() != digest:
        print("Checksum mismatch: refusing to unpack the download", file=sys.stderr)
        return 1
    dest.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(io.BytesIO(data)) as wheel:
        for name in wheel.namelist():
            if name.endswith("/"):
                continue
            if "/_native/" in name:
                target = dest / name.split("/_native/", 1)[1]
            elif "/licenses/" in name:
                target = dest / "licenses" / name.split("/licenses/", 1)[1]
            else:
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(wheel.read(name))
    marker.write_text(json.dumps({"version": VERSION, "url": url, "sha256": digest}, indent=2) + "\n")
    print(f"Unpacked to {dest}")
    return 0 if fetch_detector(dest) else 1


if __name__ == "__main__":
    sys.exit(main())
