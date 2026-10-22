#!/bin/sh
# Runs Speechify from a source checkout (after `uv sync --no-install-package
# evdev`), without re-syncing: that would try to build evdev, which Speechify
# doesn't use (see linuxkeys.py).
exec "$(cd "$(dirname "$0")" && pwd)/.venv/bin/speechify" "$@"
