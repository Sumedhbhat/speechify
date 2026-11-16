# Speechify CPU evaluation

This is a binary-only crate. It calls production `pipeline::run`,
`pipeline::compose`, and `teacher::review`; it never launches the desktop app.
The Python tools require only the standard library and execute no dataset code.

All run artifacts and downloads belong under `/workspaces/.cache-speechify/eval`.
Run from the repository root:

```bash
export CARGO_TARGET_DIR=/workspaces/.cache-speechify/target
source scripts/local-env.sh /workspaces/.cache-speechify/sysroot
export SPEECHIFY_NATIVE_DIR=/workspaces/.cache-speechify/native/lib
export PYTHONDONTWRITEBYTECODE=1
export TMPDIR=/workspaces/.cache-speechify/eval/tmp
mkdir -p "$TMPDIR"

# Run each heavy command to completion before starting the next.
systemd-run --user --scope -p MemoryMax=3500M -- python3 bench/prepare.py
systemd-run --user --scope -p MemoryMax=3500M -- cargo check -p speechify-bench
systemd-run --user --scope -p MemoryMax=3500M -- cargo build --release -p speechify-bench --locked
systemd-run --user --scope -p MemoryMax=3500M -- cargo run --release -p speechify-bench --locked -- download parakeet-tdt-0.6b-v3
systemd-run --user --scope -p MemoryMax=3500M -- cargo run --release -p speechify-bench --locked -- download whisper-large-v3-turbo

# A study directory must be new for a new experiment. These commands resume
# completed clips when repeated, but never reset or discard a study implicitly.
EVAL=/workspaces/.cache-speechify/eval
STUDY="$EVAL/parakeet"
systemd-run --user --scope -p MemoryMax=3500M -- cargo run --release -p speechify-bench --locked -- student "$EVAL/manifest.json" "$STUDY" parakeet-tdt-0.6b-v3 baseline
systemd-run --user --scope -p MemoryMax=3500M -- cargo run --release -p speechify-bench --locked -- teacher "$EVAL/manifest.json" "$STUDY" whisper-large-v3-turbo history-only
systemd-run --user --scope -p MemoryMax=3500M -- cargo run --release -p speechify-bench --locked -- student "$EVAL/manifest.json" "$STUDY" parakeet-tdt-0.6b-v3 learned
systemd-run --user --scope -p MemoryMax=3500M -- python3 bench/analyse.py "$STUDY"
systemd-run --user --scope -p MemoryMax=3500M -- cargo run --release -p speechify-bench --locked -- format bench/cases.json "$EVAL/format-rules.json"
```

The compiled binary at `$CARGO_TARGET_DIR/release/speechify-bench` accepts the
same arguments and avoids repeatedly invoking Cargo. All stages write JSON
atomically after each clip. SQLite holds the completed history/review records.
History insertion is idempotent by recording path. Baseline refuses a store
already reviewed by the teacher; learning requires pending reviews to finish.
Do not reuse one study directory for another manifest/model/settings combination.

The history/test split is fixed before recognition. General: 30/30 clips;
recurring terms: four groups, three history and three test clips each. The
selection algorithm, term candidates, seed, source revision, duration filter,
and waveform hashes are retained. The mirror lacks speaker identifiers, so
splits and bootstrap intervals are at clip level, not speaker level.

Settings: English, CPU backend, Slack/work destination, formal style, Indian
spelling and numbers, cleanup Light, language model disabled for the main speech
experiment, auto-learn on, learn-after 2. Teacher batch is 1 for per-clip
checkpointing; the ordinary default is 200. Separate stores prevent learning
between scenarios. Learned runs call real recognition again with learned
vocabulary, then real composition; they do not substitute cached student text.
The teacher sees history only. `history-only` measures teacher reference WER on
those reviews (30 general, 12 recurring); omit the option for additional teacher
transcription/scoring of test audio without writing it into the learning store.

Timings use monotonic high-resolution wall clocks. The recognizer is wrapped
for its own latency. Trimming and rules are timed as separate production-code
replays, outside `pipeline::run`; `pipeline_ms` times the real complete call,
including SQLite vocabulary/rule/example reads, excluding WAV decoding and disk
saving. Extra replay timings are **not** summed into pipeline latency. First
inference is included. `load_ms` is process-cold model opening, with uncontrolled
OS file cache, **not** a disk-cold or reboot-cold measure. `/proc/self/stat`
counters assume Linux `CLK_TCK=100`, checked in the recorded environment.

The analysis independently implements word-level Levenshtein distance. It
lowercases and deletes punctuation, retains Unicode letters/digits and word
boundaries, and applies no number or spelling normalization. Corpus WER is
sum(errors)/sum(reference words). CIs use 10,000 clip bootstrap draws, seed
20261111, nearest-rank percentiles. The paired difference is learned minus empty
store WER, so negative values favor learning. The same sampled clip indexes are
used on both sides. Better/worse counts compare per-clip edit counts. The app's
teacher agreement is also retained using the app's own normalization and
arithmetic mean, which differs from corpus WER.

Optional cleanup: run an isolated Ollama server with `OLLAMA_HOST=127.0.0.1:11434`
and `OLLAMA_MODELS=$EVAL/ollama`, under the same memory scope. Pull only
`gemma3:1b`. Stop every native inference process before loading the language
model; stop Ollama before any subsequent native run. Then run:

```bash
systemd-run --user --scope -p MemoryMax=3500M -- cargo run --release -p speechify-bench --locked -- format bench/cases.json "$EVAL/format-gemma.json" llm
```

This exercises the real prompt, HTTP client, vetting, and fallback with the
app's six-second timeout. Minimum words is zero to cover every formatting case.
`calls` retains the raw model answer and prompt, allowing fallback reasons and
vetted mistakes to be audited. The exact-match fixtures were set before model
runs: 32 adaptations of unit-test inputs to the full formal pipeline, plus 20
new cases. Expected text is never learned from observed benchmark outputs.

Before each teacher clip, the benchmark closes SQLite to checkpoint its WAL,
then saves a database backup and an in-flight marker. A restart restores that
backup and removes the corresponding unfinished JSON row, if any, before retrying
the clip. This protects the measurement from interruptions between production
`save_review` and its subsequent learning writes, without changing core.

For the teacher language-model and few-shot mechanism without concurrent ASR
memory, run this additional cleanup-only ablation after native runs finish:

```bash
systemd-run --user --scope -p MemoryMax=3500M -- cargo run --release -p speechify-bench --locked -- cleanup-study "$EVAL/manifest.json" "$STUDY" "$EVAL/parakeet-gemma"
```

It seeds fresh stores with the actual history records from native baseline
(the text typed while Ollama was absent), then calls real
`teacher::review` with the teacher's already measured speech transcripts and
Gemma 3 1B as rewriter. Held-out empty/learned calls both receive the **same
frozen baseline raw transcript**, isolating dictionary and few-shot cleanup from
ASR vocabulary effects. This is not a second measurement of speech inference.
The everyday six-word minimum and six-second HTTP timeout remain in force.
The teacher overrides the minimum and uses the desktop adapter's 180-second
review timeout (`src-tauri/src/models.rs`). Captured prompts expose
which three examples each call actually used. The teacher and student cleanup
model are the same 1B model here because of the hardware budget; the default
4B/9B cleanup pairing is not tested.

Audit the formatting and cleanup-only results:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 bench/analyse_cleanup.py "$EVAL"
```

Probe the observed quiet-speech failure after native runs finish, with no other
model running:

```bash
systemd-run --user --scope -p MemoryMax=3500M -- cargo run --release -p speechify-bench --locked -- quiet-probe parakeet-tdt-0.6b-v3 "$EVAL/audio/3274.wav" "$EVAL/quiet-probe.json"
```

This compares the production pipeline on the original waveform and at fixed 4x
gain, plus direct recognition without trimming. This diagnostic never changes
the selected waveform, baseline scores, learned store, or held-out result.
A `Silent` pipeline outcome stays in WER as an empty hypothesis. It creates no
history row, as in the app; consequently this sample has 41 reviewable records
from 42 history clips. The teacher's WER denominator includes only those 41.

Validation:

```bash
cargo fmt --all --check
systemd-run --user --scope -p MemoryMax=3500M -- cargo clippy --workspace --all-targets --locked -- -D warnings
```

Reproduce the severe teacher-example acceptance case without loading any model:

```bash
systemd-run --user --scope -p MemoryMax=3500M -- cargo run --release -p speechify-bench --locked -- example-probe "$STUDY" "$EVAL/example-probe.json"
```

This creates a fresh in-memory store with the real clip-490 student record,
replays the already measured teacher transcription through real
`teacher::review`, and shows the resulting dictionary and examples. It does not
use reference text as a recognizer output and does not change the main study.

The measured server command (after all native processes exit) was:

```bash
systemd-run --user --scope --unit=speechify-eval-ollama-20261111 -p MemoryMax=3500M -- env OLLAMA_HOST=127.0.0.1:11434 OLLAMA_MODELS="$EVAL/ollama" OLLAMA_NO_CLOUD=1 OLLAMA_NUM_PARALLEL=1 OLLAMA_MAX_LOADED_MODELS=1 OLLAMA_CONTEXT_LENGTH=2048 OLLAMA_VULKAN=0 CUDA_VISIBLE_DEVICES=-1 HIP_VISIBLE_DEVICES=-1 ROCR_VISIBLE_DEVICES=-1 /workspaces/.local/bin/ollama serve
```

Use a separate terminal for the foreground server. Pull only `gemma3:1b`, and
warm resident weights with `/api/generate` (`model=gemma3:1b`, `keep_alive=30m`).
The complete tag digest, model information, CPU/context startup log and measured
warm request are in `ollama-tags.json`, `ollama-show.json`, `ollama-serve.log`,
and `ollama-warm.json`. The retained model manifest/blobs reproduce the exact
run even if the registry tag changes. Stop the server with
`systemctl --user stop speechify-eval-ollama-20261111.scope` when finished.

The six-second formatting run timed out on all 52 cases. A separately labeled
quality diagnostic uses only the **20 previously frozen new cases**, with a
180-second request budget. It does not change the everyday timeout or the
held-out learning study:

```bash
# cases-new.json is cases.json filtered to source == "new", without text edits.
systemd-run --user --scope -p MemoryMax=3500M -- cargo run --release -p speechify-bench --locked -- format "$EVAL/cases-new.json" "$EVAL/format-gemma-quality.json" llm 180
```

For any held-out case with an accepted model rewrite, recheck its actual
six-second composition in both request orders (learned/empty, then empty/learned):

```bash
systemd-run --user --scope -p MemoryMax=3500M -- cargo run --release -p speechify-bench --locked -- cleanup-probe "$EVAL/manifest.json" "$STUDY" "$EVAL/parakeet-gemma" "$EVAL/cleanup-probe.json"
```

This diagnostic never replaces the original held-out results. It tests whether
request order/cache state changes model completion and fallback behavior.

## After-fixes evaluation

Development must use history only. This replay audit uses the saved native
student/teacher transcripts, calls production review and composition, and never
uses reference words to choose corrections. `single-sighting` is the rejected
ablation that substitutes the proposed correction itself for independent spelling
evidence; `corroborated` is the shipped mechanism. The output directory must be new.

```bash
systemd-run --user --scope -p MemoryMax=3500M -- cargo run --release -p speechify-bench --locked -- history-audit "$EVAL/manifest.json" "$EVAL/parakeet" "$EVAL/fixes-history-final"
systemd-run --user --scope -p MemoryMax=3500M -- python3 bench/analyse_fixes.py history "$EVAL/fixes-history-final"
```

After freezing implementation, repeat the native `student … baseline`,
`teacher … history-only`, and `student … learned` commands above in a fresh
`$EVAL/after-fixes` study. Do not inspect or tune to intermediate test outputs.
Score the held-out comparison once after both student passes finish:

```bash
systemd-run --user --scope -p MemoryMax=3500M -- python3 bench/analyse_fixes.py final "$EVAL/after-fixes" --original "$EVAL/parakeet"
systemd-run --user --scope -p MemoryMax=3500M -- python3 bench/analyse.py "$EVAL/after-fixes"
```

`fixes-summary.json` reports both the within-run learning effect and all changes
against the original baseline, separately for history and test, with the original
10,000-draw paired bootstrap. A change in trimming can change the new empty-store
baseline; it must not be attributed to learning.

The `format` command now retains a production rewrite circuit across cases and
injects elapsed monotonic time. After two errors or rejected answers, subsequent
back-to-back requests use rules for a minute, then probe again. Standalone
`compose` remains stateless; the desktop controller uses the persistent circuit.
Use fresh output JSON paths for the revised `format`, `quiet-probe` and
`example-probe` commands so the original evaluation stays visible.
