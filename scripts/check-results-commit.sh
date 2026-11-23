#!/usr/bin/env bash
set -euo pipefail

bot="41898282+github-actions[bot]@users.noreply.github.com"
reuse() {
  echo "$2" >&2
  echo "reuse=$1"
  exit 0
}

if [ "${GITHUB_EVENT_NAME:-}" != workflow_dispatch ]; then
  reuse false "Not a dispatched run, so everything runs."
fi
if [ "$(git log -1 --format=%ae HEAD)" != "$bot" ]; then
  reuse false "HEAD was not written by the performance job, so everything runs."
fi
changed="$(git diff --name-only HEAD^ HEAD 2> /dev/null || true)"
if [ -z "$changed" ] || grep -qv '^bench/perf/results/[^/]*\.json$' <<< "$changed"; then
  reuse false "HEAD changes more than performance results, so everything runs."
fi
parent="$(git rev-parse HEAD^)"
conclusion="$(gh api "repos/$GITHUB_REPOSITORY/commits/$parent/check-runs?check_name=Tauri%20full%20coverage%20gate&app_id=15368&filter=latest" \
  --jq '[.check_runs[] | select(.status == "completed")] | sort_by(.completed_at) | last | .conclusion // ""' || true)"
if [ "$conclusion" != success ]; then
  reuse false "The coverage gate on ${parent::7} is '${conclusion:-missing}', so everything runs."
fi
reuse true "HEAD only adds performance results, and every check passed on ${parent::7}."
