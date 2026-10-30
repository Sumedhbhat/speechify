#!/usr/bin/env python3
"""Fetch pinned native libraries without installing or invoking Python inference."""

import urllib.request
import json
import zipfile
import io
import hashlib
import pathlib
import platform

root = pathlib.Path(__file__).resolve().parents[1]
system, machine = platform.system(), platform.machine().lower()
tags = {
    ("Linux", "x86_64"): (
        "manylinux",
        "x86_64",
        "9647bff7a7486810c2778fb9b86a6ac0302688620bfad8f31b560697423641aa",
    ),
    ("Linux", "aarch64"): (
        "manylinux",
        "aarch64",
        "cffc38c389fb5933270c6b7e282c18dcb5b88798005c021a4f6644372d3f9ffa",
    ),
    ("Darwin", "arm64"): (
        "macosx",
        "arm64",
        "8b5fe9f76fcb88d64981c55837f74bd0ccba013ac7f1487216a59a1ef8dadc82",
    ),
    ("Darwin", "x86_64"): (
        "macosx",
        "x86_64",
        "a03335e8512b0f8a35ed6b3409beca6530161060dbf5f6d31615cca51edf3abd",
    ),
    ("Windows", "amd64"): (
        "win",
        "amd64",
        "8bb3e962cf9405054129a8ca9ecf9c9df7f4f044b9d6467bf94d5823ff1c9533",
    ),
}
a, b, digest = tags[(system, machine)]
meta = json.load(urllib.request.urlopen("https://pypi.org/pypi/transcribe-cpp-native/0.3.1/json"))
u = next(x for x in meta["urls"] if a in x["filename"] and b in x["filename"])
data = urllib.request.urlopen(u["url"]).read()
assert hashlib.sha256(data).hexdigest() == digest, "Native wheel checksum mismatch"
dest = root / "src-tauri/resources/native"
dest.mkdir(parents=True, exist_ok=True)
with zipfile.ZipFile(io.BytesIO(data)) as z:
    for n in z.namelist():
        if n.endswith("/"):
            continue
        if "/_native/" in n or "/licenses/" in n:
            name = (
                n.split("/_native/")[-1]
                if "/_native/" in n
                else "licenses/" + n.split("/licenses/")[-1]
            )
            p = dest / name
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_bytes(z.read(n))
(dest / "provenance.json").write_text(
    json.dumps(
        {
            "url": u["url"],
            "sha256": digest,
            "version": "0.3.1",
            "platform": system,
            "architecture": machine,
        },
        indent=2,
    )
    + "\n"
)
print(dest)
