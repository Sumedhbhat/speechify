#!/usr/bin/env python3
"""Unpack the Linux build dependencies into a folder, without root.

For machines where you cannot `apt install`. Downloads the Debian packages
Tauri needs (WebKitGTK, GTK, ALSA, the X11 test extension) and extracts them
into a sysroot. Then `source scripts/local-env.sh <sysroot>` points the
compiler and pkg-config at it. Nothing is installed system-wide.

    python3 scripts/bootstrap-linux.py [sysroot-dir]     # default: .build/sysroot
"""

import concurrent.futures
import pathlib
import subprocess
import sys

PACKAGES = [
    "libwebkit2gtk-4.1-dev",
    "libgtk-3-dev",
    "libasound2-dev",
    "libayatana-appindicator3-dev",
    "librsvg2-dev",
    "libxdo-dev",
    "libx11-dev",
    "libxtst-dev",
    "libxi-dev",
]


def main() -> int:
    root = pathlib.Path(__file__).resolve().parents[1]
    sysroot = pathlib.Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else root / ".build" / "sysroot"
    debs = sysroot.parent / "debs"
    debs.mkdir(parents=True, exist_ok=True)
    sysroot.mkdir(parents=True, exist_ok=True)
    plan = subprocess.check_output(["apt-get", "-s", "install", "--no-install-recommends", *PACKAGES], text=True)
    names = [line.split()[1] for line in plan.splitlines() if line.startswith("Inst ")]

    def fetch(name: str) -> None:
        subprocess.run(["apt-get", "download", name], cwd=debs, check=True, stdout=subprocess.DEVNULL)

    with concurrent.futures.ThreadPoolExecutor(max_workers=6) as pool:
        list(pool.map(fetch, names))
    for deb in sorted(debs.glob("*.deb")):
        subprocess.run(["dpkg-deb", "-x", str(deb), str(sysroot)], check=True)
    # Development packages link to runtime libraries that are already on the
    # system and so were not downloaded; point those links at the system copy.
    system = pathlib.Path("/usr/lib/x86_64-linux-gnu")
    repaired = 0
    for link in (sysroot / "usr/lib/x86_64-linux-gnu").glob("*.so*"):
        if link.is_symlink() and not link.exists():
            target = system / pathlib.Path(link.readlink()).name
            if target.exists():
                link.unlink()
                link.symlink_to(target)
                repaired += 1
    print(f"Extracted {len(names)} packages into {sysroot} ({repaired} links to system libraries)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
