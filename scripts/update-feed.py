#!/usr/bin/env python3
"""Writes the feed the in-app updater reads (stable.json or nightly.json).

    update-feed.py <version> <signature file> <archive url> <release url> > feed.json

The signature file is the `.app.tar.gz.sig` written by scripts/macos-package.sh.
"""

import datetime
import json
import sys


def main() -> None:
    version, signature_file, url, notes_url = sys.argv[1:5]
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
