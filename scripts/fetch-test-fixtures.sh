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

"$python" "$root/scripts/fetch-native.py"

mkdir -p "$(dirname "$model")"
check() { "$python" -c "import hashlib,sys; sys.exit(hashlib.sha256(open(sys.argv[1],'rb').read()).hexdigest() != sys.argv[2])" "$model" "$sha"; }
if ! { [ -f "$model" ] && check; }; then
  curl --fail --location --silent --show-error --retry 3 -o "$model" "$url"
  check || { echo "The test model's checksum does not match" >&2; exit 1; }
fi

# Native paths, so the same file works in Git Bash on Windows.
native="$root/src-tauri/resources/native"
if command -v cygpath >/dev/null 2>&1; then
  native="$(cygpath -m "$native")"
  model="$(cygpath -m "$model")"
fi
cat > "$root/.build/test-env.sh" <<EOF
export SPEECHIFY_NATIVE_DIR="$native"
export SPEECHIFY_TEST_MODEL="$model"
EOF
echo "Wrote .build/test-env.sh"
