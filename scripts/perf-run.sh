#!/usr/bin/env bash
set -euo pipefail

perf="$1"
llm_dir="$2"
output="$3"
port="${PERF_LLM_PORT:-8089}"
source "$llm_dir/env.sh"

"$PERF_LLAMA_SERVER" -m "$PERF_LLM_FILE" --host 127.0.0.1 --port "$port" -c 4096 -ngl 0 -np 1 \
  > "$llm_dir/server.log" 2>&1 &
server=$!
trap 'kill "$server" 2>/dev/null || true' EXIT

for _ in $(seq 1 300); do
  if curl -sf "http://127.0.0.1:$port/health" > /dev/null; then
    break
  fi
  if ! kill -0 "$server" 2>/dev/null; then
    cat "$llm_dir/server.log" >&2
    echo "The language model server stopped before it was ready" >&2
    exit 1
  fi
  sleep 1
done

PERF_LLM_URL="http://127.0.0.1:$port/v1" "$perf" bench/scenarios.json "$output"
