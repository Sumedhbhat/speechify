#!/usr/bin/env bash
# Fast, repeatable checks for a structural change. Native coverage runs separately.
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ -z "${PYTHON:-}" ]]; then
  case "$(uname -s)" in
    MINGW*|MSYS*|CYGWIN*) PYTHON=python ;;
    *) PYTHON=python3 ;;
  esac
fi

npm run format:check
npm run test:coverage
npm run build
"$PYTHON" tests/coverage-gate.test.py
"$PYTHON" scripts/check-platform-adapters.py
"$PYTHON" scripts/test-optional-installer.py
"$PYTHON" scripts/prepare-native.py
cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features
cargo test --manifest-path src-tauri/Cargo.toml --locked --features behavioral-tests
