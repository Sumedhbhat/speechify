#!/usr/bin/env bash
set -euo pipefail

check="$1"
bot="41898282+github-actions[bot]@users.noreply.github.com"

if [ "$(git log -1 --format=%ae HEAD)" != "$bot" ]; then
  echo "HEAD was not written by the results job, so it needs the full CI run." >&2
  exit 1
fi
changed="$(git diff --name-only HEAD^ HEAD)"
if [ -z "$changed" ] || grep -v '^bench/perf/results/[^/]*\.json$' <<< "$changed"; then
  echo "HEAD changes more than performance results, so it needs the full CI run." >&2
  exit 1
fi
parent="$(git rev-parse HEAD^)"
conclusion="$(bash scripts/wait-for-checks.sh "$parent" "$check")"
if [ "$conclusion" != success ]; then
  echo "$check concluded $conclusion on ${parent::7}, the commit these results were measured on." >&2
  exit 1
fi
echo "$check passed on ${parent::7}, and HEAD only adds performance results."
