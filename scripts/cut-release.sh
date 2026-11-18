#!/usr/bin/env bash
# Starts a stable release line and retires the one before it:
#   scripts/cut-release.sh <major>.<minor>
#
# Only one release/* branch takes changes at a time. The "release: retired"
# ruleset makes every release/* branch read-only except the one it excludes,
# so this script first moves that exclusion to release/<major>.<minor>
# (locking the previous line) and then creates the branch from main. The root
# Cargo.toml on main must already be at <major>.<minor>.x. Creating release/*
# branches needs the repository admin role. See docs/releases.md.
set -euo pipefail

line="${1:-}"
if [[ ! "$line" =~ ^[0-9]+\.[0-9]+$ ]]; then
  echo "Usage: scripts/cut-release.sh <major>.<minor>, such as 0.2" >&2
  exit 1
fi
branch="release/$line"
repo="$(gh repo view --json nameWithOwner --jq .nameWithOwner)"

if gh api "repos/$repo/branches/$branch" >/dev/null 2>&1; then
  echo "$branch exists already. Ship fixes to it with a pull request." >&2
  exit 1
fi

sha="$(gh api "repos/$repo/commits/main" --jq .sha)"
version="$(gh api -H "Accept: application/vnd.github.raw" "repos/$repo/contents/Cargo.toml?ref=$sha" |
  sed -n 's/^version = "\(.*\)"$/\1/p' | head -n 1)"
case "$version" in
  "$line".*) ;;
  *)
    echo "main is at version $version. Set it to $line.0 through a pull request first." >&2
    exit 1
    ;;
esac

id="$(gh api "repos/$repo/rulesets" --jq '.[] | select(.name == "release: retired") | .id')"
if [ -z "$id" ]; then
  echo "The \"release: retired\" ruleset is missing. Set it up as docs/releases.md describes." >&2
  exit 1
fi
current="$(gh api "repos/$repo/rulesets/$id" --jq '.conditions.ref_name.exclude[0] // ""')"
current="${current#refs/heads/}"

if [ "$current" != "$branch" ]; then
  newest="$(printf '%s\n%s\n' "${current#release/}" "$line" | sort -V | tail -n 1)"
  if [ -n "$current" ] && [ "$newest" != "$line" ]; then
    echo "The current release line is $current; $branch would go backwards." >&2
    exit 1
  fi
  gh api -X PUT "repos/$repo/rulesets/$id" --input - >/dev/null <<EOF
{"conditions": {"ref_name": {"include": ["refs/heads/release/*"], "exclude": ["refs/heads/$branch"]}}}
EOF
  echo "Retired ${current:-nothing}; $branch is now the only release branch that takes changes."
fi

gh api -X POST "repos/$repo/git/refs" -f ref="refs/heads/$branch" -f sha="$sha" >/dev/null
echo "Created $branch at ${sha:0:7} ($version). The release workflow is publishing v$version."
echo "Next: set main's version to the following minor release through a pull request."
