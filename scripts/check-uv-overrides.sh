#!/usr/bin/env bash
# Checks every uv override in the ledger. With --open-prs, opens an auto-merging PR for each
# override that is no longer needed, and an issue for each override older than STALE_DAYS.
#
#   NEEDED     without the override, a vulnerable version would be installed
#   WAITING    safe to remove, but it would pull in a version younger than MIN_AGE_DAYS
#   REMOVABLE  safe to remove now
#
# Overrides live in [tool.uv] override-dependencies in pyproject.toml, one requirement per line.
# Needs: uv, yq, jq, curl, osv-scanner, git, gh.
set -euo pipefail
shopt -s inherit_errexit

LEDGER=.github/dependency-overrides.yml
MIN_AGE_DAYS=${MIN_AGE_DAYS:-14}
STALE_DAYS=${STALE_DAYS:-90}
OPEN_PRS=false
[[ ${1:-} == --open-prs ]] && OPEN_PRS=true
now=$(date +%s)

# entry <pkg> <yq expression>: reads one field of the package's ledger entry.
entry() { PKG=$1 yq -r ".overrides[] | select(.ecosystem == \"uv\" and .package == strenv(PKG)) | .$2" "$LEDGER"; }

days_since() { echo $(( (now - $(date -d "$1" +%s)) / 86400 )); }

py() { uv run --quiet --no-project --with packaging python - "$@"; }

# Deletes the override and its ledger entry, then re-resolves the lockfile without installing.
drop_override() {
  py "$1" <<'PY'
import re, sys
from pathlib import Path
from packaging.requirements import Requirement
from packaging.utils import canonicalize_name

pkg = canonicalize_name(sys.argv[1])
path = Path("pyproject.toml")
text = path.read_text()
block = re.search(r"(?ms)^override-dependencies\s*=\s*\[\n(.*?)^\]\n", text)
kept = [line for line in block.group(1).splitlines(keepends=True)
        if not line.strip().strip(",").strip('"')
        or canonicalize_name(Requirement(line.strip().rstrip(",").strip('"')).name) != pkg]
if any(line.strip() for line in kept):
    text = text[:block.start(1)] + "".join(kept) + text[block.end(1):]
else:
    text = text[:block.start()] + text[block.end():]
path.write_text(text)
PY
  PKG=$1 yq -i 'del(.overrides[] | select(.ecosystem == "uv" and .package == strenv(PKG)))' "$LEDGER"
  uv lock --quiet
}

# Prints every version of <pkg> in uv.lock.
resolved_versions() {
  py "$1" <<'PY'
import sys, tomllib
from packaging.utils import canonicalize_name

pkg = canonicalize_name(sys.argv[1])
lock = tomllib.load(open("uv.lock", "rb"))
for p in lock.get("package", []):
    if canonicalize_name(p["name"]) == pkg and "version" in p:
        print(p["version"])
PY
}

at_least() { py "$1" "$2" <<'PY'
import sys
from packaging.version import Version
sys.exit(0 if Version(sys.argv[1]) >= Version(sys.argv[2]) else 1)
PY
}

# Prints the status for <pkg> <fixed_in>. Edits files, so run it inside a scratch worktree.
check() {
  local pkg=$1 fixed=$2 audit published v status=REMOVABLE
  drop_override "$pkg"

  # osv-scanner exits non-zero whenever anything is vulnerable, so capture first, then inspect.
  audit=$(osv-scanner scan source -L uv.lock --format json 2>/dev/null || true)
  if jq -e --arg p "$pkg" \
      '[.results[]?.packages[]? | select((.package.name | ascii_downcase) == ($p | ascii_downcase)) | .vulnerabilities[]?] | length > 0' \
      <<<"$audit" >/dev/null; then
    echo NEEDED; return
  fi

  while read -r v; do
    [[ -z $v ]] && continue
    at_least "$v" "$fixed" || { echo NEEDED; return; }
    published=$(curl -fsSL "https://pypi.org/pypi/$pkg/$v/json" | jq -r '.urls[0].upload_time_iso_8601')
    if (( $(days_since "$published") < MIN_AGE_DAYS )); then
      status=WAITING
    fi
  done < <(resolved_versions "$pkg" | sort -u)
  echo "$status"
}

open_pr() {
  local pkg=$1 fixed=$2 branch="deps/remove-override-$1"
  if git ls-remote --exit-code --heads origin "$branch" >/dev/null; then
    echo "  $branch already exists, skipping"; return
  fi
  git switch --quiet -c "$branch"
  git commit --quiet -am "chore(deps): remove override for $pkg" \
    -m "Without the override, every resolved $pkg is >= $fixed, osv-scanner reports nothing for it, and every newly resolved version is at least $MIN_AGE_DAYS days old."
  git push --quiet origin "$branch"
  gh pr create --head "$branch" --title "chore(deps): remove override for $pkg" \
    --body "Opened by the dependency-overrides workflow. Upstream now resolves \`$pkg\` to a fixed version (>= $fixed) on its own, so the override and its ledger entry are no longer needed."
  gh pr merge --auto --squash "$branch"
}

flag_stale() {
  local pkg=$1 title="Stale dependency override: $1"
  if [[ -n $(gh issue list --state open --search "\"$title\" in:title" --json number -q '.[].number') ]]; then
    return
  fi
  gh issue create --title "$title" \
    --body "The override for \`$pkg\` was added on $2 and is still needed. Owner: $3. Check whether $4 has a fix upstream, or replace it."
}

mapfile -t pkgs < <(yq -r '.overrides[] | select(.ecosystem == "uv") | .package' "$LEDGER")

for pkg in "${pkgs[@]}"; do
  fixed=$(entry "$pkg" fixed_in)
  added=$(entry "$pkg" added)
  owner=$(entry "$pkg" owner)
  parents=$(entry "$pkg" 'parents | join(", ")')

  dir=$(mktemp -d)
  git worktree add --quiet --detach "$dir" HEAD
  pushd "$dir" >/dev/null
  status=$(check "$pkg" "$fixed")
  echo "$pkg: $status (override added $added)"
  if $OPEN_PRS; then
    if [[ $status == REMOVABLE ]]; then
      open_pr "$pkg" "$fixed"
    elif [[ $status == NEEDED ]] && (( $(days_since "$added") >= STALE_DAYS )); then
      flag_stale "$pkg" "$added" "$owner" "$parents"
    fi
  fi
  popd >/dev/null
  git worktree remove --force "$dir"
done
