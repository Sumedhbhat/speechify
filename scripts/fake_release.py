"""Serve installers as if they were a GitHub release, to test updating.

    python scripts/fake_release.py <folder> <version> [port]

Publishes every file in <folder> as release v<version> at
http://127.0.0.1:<port> (default 8765), with a SHA256SUMS, the way the
Release workflow does. Point Speechify at it with

    SPEECHIFY_UPDATE_API=http://127.0.0.1:8765 speechify update

GitHub renames "~" in uploaded names to "."; so does this, so the .deb is
served under the name a real release gives it.
"""

from __future__ import annotations

import functools
import hashlib
import http.server
import json
import sys
from pathlib import Path


def publish(folder: Path, version: str, port: int) -> None:
    built = sorted(f for f in folder.iterdir() if f.is_file() and f.name not in ("releases", "SHA256SUMS"))
    # Listed under the names build.sh gave them, like the real SHA256SUMS...
    (folder / "SHA256SUMS").write_text("".join(
        f"{hashlib.sha256(f.read_bytes()).hexdigest()}  {f.name}\n" for f in built))
    # ...and served under GitHub's names.
    files = [f.rename(f.with_name(f.name.replace("~", "."))) for f in built]
    base = f"http://127.0.0.1:{port}"
    release = {
        "tag_name": f"v{version}",
        "prerelease": "-" in version,
        "draft": False,
        "html_url": f"{base}/v{version}",
        "assets": [{"name": f.name, "browser_download_url": f"{base}/{f.name}"}
                   for f in files + [folder / "SHA256SUMS"]],
    }
    # GET /releases (the query string is ignored), which pre-release builds
    # ask for. Stable builds ask for /releases/latest, which isn't served.
    (folder / "releases").write_text(json.dumps([release]))
    print(f"Serving Speechify {version} at {base}: {', '.join(f.name for f in files)}", flush=True)


def main() -> None:
    folder, version = Path(sys.argv[1]), sys.argv[2]
    port = int(sys.argv[3]) if len(sys.argv) > 3 else 8765
    publish(folder, version, port)
    handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=str(folder))
    http.server.ThreadingHTTPServer(("127.0.0.1", port), handler).serve_forever()


if __name__ == "__main__":
    main()
