#!/usr/bin/env python3
"""Writes the feed the in-app updater reads (stable.json or nightly.json).

    update-feed.py <version> <signature file> <archive url> <release url> [current feed] > feed.json

The signature file is the `.app.tar.gz.sig` written by scripts/macos-package.sh.
With the feed that is published now, it exits with status 3 and writes nothing
unless <version> is newer, so an older patch release or a rerun of an old build
never takes the feed backwards.
"""

import datetime
import json
import sys


def order(version: str) -> tuple:
    """Semantic-version order: 0.1.0-nightly.999 < 0.1.0-nightly.1000 < 0.1.0."""
    core, _, pre = version.partition("-")
    numbers = tuple(int(part) for part in core.split("."))
    if not pre:
        return numbers, (1,)
    parts = tuple((0, int(p), "") if p.isdigit() else (1, 0, p) for p in pre.split("."))
    return numbers, (0, parts)


def main() -> None:
    version, signature_file, url, notes_url = sys.argv[1:5]
    if len(sys.argv) > 5:
        try:
            with open(sys.argv[5], encoding="utf-8") as f:
                current = json.load(f)["version"]
        except (OSError, ValueError, KeyError):
            current = None
        if current and order(current) >= order(version):
            print(f"The feed already offers {current}; not replacing it with {version}.", file=sys.stderr)
            sys.exit(3)
    with open(signature_file, encoding="utf-8") as f:
        signature = f.read().strip()
    feed = {
        "version": version,
        "notes": f"Release notes: {notes_url}",
        "pub_date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "platforms": {"darwin-aarch64": {"signature": signature, "url": url}},
    }
    json.dump(feed, sys.stdout, indent=2)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
