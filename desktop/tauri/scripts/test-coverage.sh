#!/usr/bin/env bash
# One reproducible run of the interface, full desktop backend, and worker CLI.
set -euo pipefail
cd "$(dirname "$0")/.."
PYTHON="${PYTHON:-python3}"
if [[ "$(uname -s)" != Linux && "${SPEECHIFY_TEST_DISPOSABLE_DESKTOP:-}" != 1 ]]; then
  echo 'Native clipboard coverage requires a disposable desktop. Use the CI coverage job.' >&2
  exit 1
fi
"$PYTHON" scripts/check-coverage.py --snapshot .build/coverage-source.json
"$PYTHON" tests/coverage-gate.test.py
npm run test:coverage
npm run build
"$PYTHON" scripts/prepare-native.py
# Exercise the built frontend in the debug coverage executable.
export TAURI_CONFIG='{"build":{"devUrl":null}}'
cargo llvm-cov clean --workspace --manifest-path src-tauri/Cargo.toml
if [[ "$(uname -s)" == Linux ]]; then
  env -u WAYLAND_DISPLAY XDG_SESSION_TYPE=x11 ALSA_CONFIG_PATH="$PWD/tests/fixtures/alsa.conf" SPEECHIFY_TEST_SYNTHETIC_ALSA=1 xvfb-run -a \
    cargo llvm-cov --manifest-path src-tauri/Cargo.toml --locked --no-report -- --include-ignored
else
  cargo llvm-cov --manifest-path src-tauri/Cargo.toml --locked --no-report -- --include-ignored
fi
cargo llvm-cov run --manifest-path src-tauri/Cargo.toml --locked --no-clean --json --output-path coverage/rust.json -- diagnostics
LLVM_PROFILE_FILE="$PWD/src-tauri/target/llvm-cov-target/cli-%p-%m.profraw" \
  "$PYTHON" tests/cli.test.py src-tauri/target/llvm-cov-target/debug/speechify-tauri-candidate
cargo llvm-cov report --manifest-path src-tauri/Cargo.toml --json --output-path coverage/rust.json
cargo llvm-cov report --manifest-path src-tauri/Cargo.toml --lcov --output-path coverage/rust.info
"$PYTHON" scripts/check-coverage.py --compare-source .build/coverage-source.json --snapshot coverage/source.json
if [[ "${1:-}" != --collect-only ]]; then
  "$PYTHON" scripts/check-coverage.py
fi
