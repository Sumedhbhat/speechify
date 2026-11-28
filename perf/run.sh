#!/usr/bin/env bash
set -euo pipefail

perf="$1"
llm_dir="$2"
output="$3"
port="${PERF_LLM_PORT:-8089}"
python="${PYTHON:-python3}"
source "$llm_dir/env.sh"

# An Ollama of its own: its own port and model folder, one request at a time, the model
# kept loaded, and the 4096-token context the gate has always used.
export OLLAMA_HOST="127.0.0.1:$port"
export OLLAMA_MODELS="$llm_dir/models"
export OLLAMA_NUM_PARALLEL=1
export OLLAMA_MAX_LOADED_MODELS=1
export OLLAMA_KEEP_ALIVE=-1
export OLLAMA_CONTEXT_LENGTH=4096
export OLLAMA_NO_CLOUD=1
url="http://127.0.0.1:$port"

"$PERF_OLLAMA" serve > "$llm_dir/server.log" 2>&1 &
server=$!
stop() {
  # Under Git Bash, kill does not reach the runner processes Ollama starts.
  if [ -r "/proc/$server/winpid" ]; then
    taskkill //F //T //PID "$(cat "/proc/$server/winpid")" > /dev/null 2>&1 || true
  fi
  kill "$server" 2>/dev/null || true
}
trap stop EXIT

ready=false
for _ in $(seq 1 300); do
  if curl -sf "$url/api/version" > /dev/null; then
    ready=true
    break
  fi
  if ! kill -0 "$server" 2>/dev/null; then
    cat "$llm_dir/server.log" >&2
    echo "The language model server stopped before it was ready" >&2
    exit 1
  fi
  sleep 1
done

if [ "$ready" != true ]; then
  cat "$llm_dir/server.log" >&2
  echo "The language model server was not ready after 5 minutes" >&2
  exit 1
fi

pulled=false
for attempt in 1 2 3; do
  if curl -sSf "$url/api/pull" -d "{\"model\": \"$PERF_LLM_MODEL\", \"stream\": false}" > /dev/null; then
    pulled=true
    break
  fi
  echo "Pulling $PERF_LLM_MODEL failed; trying again" >&2
  sleep $((10 * attempt))
done
if [ "$pulled" != true ]; then
  echo "Could not pull $PERF_LLM_MODEL after 3 attempts" >&2
  exit 1
fi
PERF_LLM_DIGEST="$("$python" "$(dirname "$0")/fetch-ollama.py" digest "$llm_dir")"
export PERF_LLM_DIGEST
echo "Measuring $PERF_LLM_MODEL ($PERF_LLM_DIGEST)" >&2

mkdir -p "$(dirname "$output")"
PERF_LLM_URL="$url" "$perf" "$(dirname "$0")/scenarios.json" "$output"
