#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
output="${1:-.build/perf/local.json}"
export PYTHON="${PYTHON:-python3}"

bash scripts/fetch-test-fixtures.sh "$PYTHON"
"$PYTHON" perf/fetch-ollama.py .build/llm
cargo build --release --locked -p speechify-perf --bin perf
source .build/test-env.sh
bash perf/run.sh "${CARGO_TARGET_DIR:-target}/release/perf" .build/llm "$output"
