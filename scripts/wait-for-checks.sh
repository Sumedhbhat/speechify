#!/usr/bin/env bash
set -euo pipefail

sha="$1"
check="$2"
name="$(jq -rn --arg v "$check" '$v | @uri')"

for _ in $(seq 1 100); do
  state="$(gh api "repos/$GITHUB_REPOSITORY/commits/$sha/check-runs?check_name=$name&app_id=15368&filter=latest" \
    --jq '.check_runs | sort_by(.started_at) | last | if . == null then "missing" else .status + " " + (.conclusion // "") end')"
  case "$state" in
    "completed "*)
      echo "${state#completed }"
      exit 0
      ;;
  esac
  echo "$check on ${sha::7}: $state" >&2
  sleep 30
done
echo "Timed out waiting for $check on $sha" >&2
exit 1
