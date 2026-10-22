#!/bin/sh
# Runs the environment scripts/install.sh built, without re-syncing it: that
# would try to build evdev, which Speechify doesn't use (see install.sh).
exec "$(cd "$(dirname "$0")" && pwd)/.venv/bin/speechify" "$@"
