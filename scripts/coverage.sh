#!/usr/bin/env bash
# The coverage gate: every line of the core and of the interface must be
# exercised by a test. Reports land in coverage/.
#
# What is and is not measured:
#   core/       all application logic                      100% of lines required
#   src/        the interface (TypeScript)                 100% required (vitest.config.ts)
#   os/         adapters that drive real devices           compiled and linted, not measurable headless
#   src-tauri/  window, tray and thread wiring             compiled and linted, not measurable headless
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

[ -f .build/test-env.sh ] || bash scripts/fetch-test-fixtures.sh
source .build/test-env.sh
export SPEECHIFY_REQUIRE_NATIVE=1

mkdir -p coverage
ignore='(^|/)testing\.rs$'
cargo llvm-cov --package speechify-core --locked --ignore-filename-regex "$ignore" --lcov --output-path coverage/core.lcov
cargo llvm-cov report --package speechify-core --ignore-filename-regex "$ignore" | tail -n 40

# Every executable line must have run at least once.
missed="$(awk -F'[:,]' '/^SF:/ { file = substr($0, 4) } /^DA:/ && $3 == 0 { print file ":" $2 }' coverage/core.lcov)"
if [ -n "$missed" ]; then
  echo "Lines never run by a test:" >&2
  echo "$missed" >&2
  exit 1
fi
echo "Core: every line is covered."

npm run test:coverage
