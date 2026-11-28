#!/usr/bin/env bash
# The coverage gate: every line of the engine, the controller and the
# interface must be exercised by a test. Reports land in coverage/.
#
# What is and is not measured:
#   core/        recognition, cleanup, storage, learning    100% of lines required
#   controller/  shortcuts, sessions, the controller        100% of lines required
#   ui/src/      the interface (TypeScript)                 100% required (ui/vitest.config.ts)
#   os/          adapters that drive real devices           compiled and linted, not measurable headless
#   src-tauri/   window, tray and thread wiring             compiled and linted, not measurable headless
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
parts=("$@")
[ ${#parts[@]} -gt 0 ] || parts=(core controller ui)
mkdir -p coverage

rust() {
  local dir="$1" package="$2" ignore='(^|/)testing\.rs$' others
  others="$(printf '%s\n' core controller os bench perf src-tauri | grep -vx "$dir" | paste -sd'|')"
  ignore="$ignore|/($others)/src/"
  cargo llvm-cov --package "$package" --locked --ignore-filename-regex "$ignore" --lcov --output-path "coverage/$dir.lcov"
  cargo llvm-cov report --package "$package" --ignore-filename-regex "$ignore" | tail -n 40

  # Every executable line must have run at least once.
  local missed
  missed="$(awk -F'[:,]' '/^SF:/ { file = substr($0, 4) } /^DA:/ && $3 == 0 { print file ":" $2 }' "coverage/$dir.lcov")"
  if [ -n "$missed" ]; then
    echo "Lines in $dir never run by a test:" >&2
    echo "$missed" >&2
    exit 1
  fi
  echo "$dir: every line is covered."
}

for part in "${parts[@]}"; do
  case "$part" in
    core)
      # Always run it: it is cheap when the cache is valid, and it rewrites
      # test-env.sh if the checkout moved or a model file went missing.
      bash scripts/fetch-test-fixtures.sh
      source .build/test-env.sh
      export SPEECHIFY_REQUIRE_NATIVE=1
      rust core speechify-core
      ;;
    controller) rust controller speechify-controller ;;
    ui) npm run test:coverage ;;
    *)
      echo "Unknown part: $part (expected core, controller or ui)" >&2
      exit 2
      ;;
  esac
done
