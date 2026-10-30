#!/usr/bin/env python3
"""Package the actual host release binary; never rename it as another OS."""

import pathlib
import platform
import shutil
import hashlib
import tarfile
import os

root = pathlib.Path(__file__).resolve().parents[1]
# Release CI passes the stamped version; local builds are named "dev".
version = os.environ.get("SPEECHIFY_VERSION", "dev")
name = f"Speechify-{version}-{platform.system().lower()}-{platform.machine()}"
artifacts = root / "artifacts"
stage = artifacts / name
if stage.exists():
    shutil.rmtree(stage)
stage.mkdir(parents=True, exist_ok=True)
binary = "speechify" + (".exe" if platform.system() == "Windows" else "")
shutil.copy2(root / "src-tauri/target/release" / binary, stage / binary)
shutil.copytree(
    root / "src-tauri/resources",
    stage / "resources",
    dirs_exist_ok=True,
    ignore=shutil.ignore_patterns("__pycache__", "*.pyc"),
)
shutil.copy2(root / "README.md", stage / "README.md")
(stage / "scripts").mkdir()
shutil.copy2(root / "scripts/setup-exception.py", stage / "scripts/setup-exception.py")
(stage / "VERSION").write_text(version + "\n")
manifest = []
for p in sorted(stage.rglob("*")):
    if p.is_file():
        manifest.append(
            hashlib.sha256(p.read_bytes()).hexdigest() + "  " + str(p.relative_to(stage))
        )
(stage / "SHA256SUMS").write_text("\n".join(manifest) + "\n")
archive = artifacts / (name + ".tar.gz")
with tarfile.open(archive, "w:gz") as tar:
    tar.add(stage, arcname=name)
print(archive)
