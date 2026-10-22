#!/usr/bin/env bash
# Builds dist/speechify-<version>.zip, the archive the installer downloads.
#
#   scripts/build-bundle.sh 0.2.0-nightly.57
#
# The archive holds the source, uv.lock and a VERSION file. The installer runs
# `uv sync --frozen` on it, so users get exactly the locked dependencies.
set -euo pipefail

version=${1:?usage: build-bundle.sh <version>}
name="speechify-$version"
root=$(cd "$(dirname "$0")/.." && pwd)
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT

mkdir -p "$stage/$name/scripts"
cp -R "$root/src" "$stage/$name/src"
cp "$root/pyproject.toml" "$root/uv.lock" "$root/README.md" "$root/start.bat" "$root/start.sh" "$stage/$name/"
cp "$root/scripts/finetune.py" "$root/scripts/install.ps1" "$root/scripts/install.sh" "$stage/$name/scripts/"
cp -R "$root/docs" "$stage/$name/docs"
find "$stage" -name __pycache__ -type d -prune -exec rm -rf {} +

# The app reads src/speechify/VERSION; the top-level copy is for people.
echo "$version" > "$stage/$name/VERSION"
echo "$version" > "$stage/$name/src/speechify/VERSION"

mkdir -p "$root/dist"
rm -f "$root/dist/$name.zip"
(cd "$stage" && zip -qr "$root/dist/$name.zip" "$name")
echo "dist/$name.zip"
