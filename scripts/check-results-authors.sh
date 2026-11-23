#!/usr/bin/env bash
set -euo pipefail

base="$1"
head="$2"
bot="41898282+github-actions[bot]@users.noreply.github.com"
failed=0
while IFS= read -r file; do
  [ -n "$file" ] || continue
  written="$(git log -1 -F --format=%H --author="$bot" "$base..$head" -- "$file")"
  now="$(git rev-parse -q --verify "$head:$file" || true)"
  wrote=""
  if [ -n "$written" ]; then
    wrote="$(git rev-parse -q --verify "$written:$file" || true)"
  fi
  if [ -z "$written" ] || [ "$now" != "$wrote" ]; then
    echo "::error file=$file::Only CI writes $file. Drop the change made to it on this branch."
    failed=1
  fi
done < <(git diff --no-renames --name-only "$base" "$head" -- bench/perf/results)
exit "$failed"
