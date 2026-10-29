#!/usr/bin/env bash
# One reproducible run of the interface, full desktop backend, and worker CLI.
set -euo pipefail
cd "$(dirname "$0")/.."
PYTHON="${PYTHON:-python3}"
"$PYTHON" tests/coverage-gate.test.py
npm run test:coverage
npm run build
"$PYTHON" scripts/prepare-native.py
cargo llvm-cov clean --workspace --manifest-path src-tauri/Cargo.toml
cargo llvm-cov --manifest-path src-tauri/Cargo.toml --locked --no-report
cargo llvm-cov run --manifest-path src-tauri/Cargo.toml --locked --no-clean --json --output-path coverage/rust.json -- diagnostics
LLVM_PROFILE_FILE="$PWD/src-tauri/target/llvm-cov-target/cli-%p-%m.profraw" \
  "$PYTHON" tests/cli.test.py src-tauri/target/llvm-cov-target/debug/speechify-tauri-candidate
cargo llvm-cov report --manifest-path src-tauri/Cargo.toml --json --output-path coverage/rust.json
cargo llvm-cov report --manifest-path src-tauri/Cargo.toml --lcov --output-path coverage/rust.info
if [[ "${1:-}" != --collect-only ]]; then
  "$PYTHON" scripts/check-coverage.py
fi
