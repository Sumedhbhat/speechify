#!/usr/bin/env python3
"""Cache the smallest pinned GGUF for the real model-reuse behavior test."""

import hashlib
import json
from pathlib import Path
import urllib.request

root = Path(__file__).resolve().parents[1]
catalog = json.loads((root / "src-tauri/resources/catalog.json").read_text())
artifact = next(model["gguf"] for model in catalog if model["id"] == "moonshine-base")
destination = root / ".build/fixtures/moonshine-base.gguf"


def matches(path):
    if not path.is_file():
        return False
    with path.open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest() == artifact["sha256"]


if not matches(destination):
    destination.parent.mkdir(parents=True, exist_ok=True)
    partial = destination.with_suffix(".partial")
    url = f"https://huggingface.co/{artifact['repo']}/resolve/{artifact['revision']}/{artifact['file']}"
    with urllib.request.urlopen(url, timeout=60) as response, partial.open("wb") as file:
        while chunk := response.read(1024 * 1024):
            file.write(chunk)
    if not matches(partial):
        partial.unlink()
        raise SystemExit("Pinned test model checksum mismatch")
    partial.replace(destination)
print(destination)
