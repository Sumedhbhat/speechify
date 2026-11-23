#!/usr/bin/env bash
set -euo pipefail

bot="41898282+github-actions[bot]@users.noreply.github.com"
edits="$(git log --no-merges --format='%h%x09%ae%x09%s' "$1..$2" -- bench/perf/results)"
others="$(awk -F'\t' -v bot="$bot" '$2 != bot' <<< "$edits")"
if [ -n "$others" ]; then
  echo "::error::Only the Performance workflow writes bench/perf/results. Drop these changes from the branch:"
  echo "$others"
  exit 1
fi
