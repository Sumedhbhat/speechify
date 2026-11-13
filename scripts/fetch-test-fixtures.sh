#!/usr/bin/env bash
# Fetches what the speech-engine tests need: the pinned native runtime and a
# tiny Whisper model. Writes .build/test-env.sh, which points the tests at them.
#
#   bash scripts/fetch-test-fixtures.sh [python-command]
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
python="${1:-python3}"
url="${TEST_MODEL_URL:-https://huggingface.co/handy-computer/whisper-tiny.en-gguf/resolve/9dd46e00ef0577e354e658c07b3ac5580e9cd211/whisper-tiny.en-Q8_0.gguf}"
sha="${TEST_MODEL_SHA256:-e8c9b73c06344307d8b346e07fbe93dd88d894627854bcff31523f1ce44394fa}"
model="$root/.build/models/whisper-tiny.en-Q8_0.gguf"
# A small transducer model: the only family that reports per-word confidence.
token_url="${TEST_TOKEN_MODEL_URL:-https://huggingface.co/handy-computer/parakeet-tdt_ctc-110m-gguf/resolve/1c3f0d5378fdd358905cc410a93ed9e64f52d170/parakeet-tdt_ctc-110m-Q4_K_M.gguf}"
token_sha="${TEST_TOKEN_MODEL_SHA256:-486414fd90185a8c8a4ced7c123cfb133ff4f7958426c6b8bd9049946b56b448}"
token_model="$root/.build/models/parakeet-tdt_ctc-110m-Q4_K_M.gguf"

"$python" "$root/scripts/fetch-native.py"

mkdir -p "$(dirname "$model")"
check() { "$python" -c "import hashlib,sys; sys.exit(hashlib.sha256(open(sys.argv[1],'rb').read()).hexdigest() != sys.argv[2])" "$1" "$2"; }
fetch() {
  if ! { [ -f "$1" ] && check "$1" "$3"; }; then
    curl --fail --location --silent --show-error --retry 3 -o "$1" "$2"
    check "$1" "$3" || { echo "The checksum of $1 does not match" >&2; exit 1; }
  fi
}
fetch "$model" "$url" "$sha"
fetch "$token_model" "$token_url" "$token_sha"

# Native paths, so the same file works in Git Bash on Windows.
native="$root/src-tauri/resources/native"
if command -v cygpath >/dev/null 2>&1; then
  native="$(cygpath -m "$native")"
  model="$(cygpath -m "$model")"
  token_model="$(cygpath -m "$token_model")"
fi
cat > "$root/.build/test-env.sh" <<EOF
export SPEECHIFY_NATIVE_DIR="$native"
export SPEECHIFY_TEST_MODEL="$model"
export SPEECHIFY_TEST_TOKEN_MODEL="$token_model"
EOF
echo "Wrote .build/test-env.sh"
