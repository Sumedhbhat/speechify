# Testing

## Commands

```
bash scripts/fetch-test-fixtures.sh   # speech runtime + two test models (46 MB and 90 MB), writes .build/test-env.sh
source .build/test-env.sh
cargo test --workspace --locked       # Rust tests
npm test                              # interface tests (Vitest, jsdom)
bash scripts/coverage.sh              # the coverage gate
```

On Windows run the script under Git Bash and pass `python`: `bash scripts/fetch-test-fixtures.sh python`.

`fetch-test-fixtures.sh` runs `fetch-native.py`, then downloads two models pinned by revision and SHA-256 and cached in `.build/models`: Whisper tiny.en (Q8_0, 46 MB) and Parakeet TDT/CTC 110M (Q4_K_M, 90 MB, the only small model that reports per-word confidence). It writes `.build/test-env.sh`, which sets `SPEECHIFY_NATIVE_DIR`, `SPEECHIFY_TEST_MODEL` and `SPEECHIFY_TEST_TOKEN_MODEL`. Rerunning it is cheap: files that match their checksums are not downloaded again.

The speech-engine tests in `core/src/native.rs` run against the real runtime and those models, using `core/tests/fixtures/jfk.wav`. Without the variables they skip, unless `SPEECHIFY_REQUIRE_NATIVE=1` is set, in which case they fail. CI and `coverage.sh` set it, so a missing runtime cannot pass quietly.

Useful narrower runs:

```
cargo test -p speechify-core format        # the formatter
cargo test -p speechify-core session
npx vitest run tests/format.test.ts
npx vitest                                 # watch mode
```

## What is tested, and how

- **`core/`**: unit tests sit beside the code. The formatter passes are tables of input and output pairs. `session.rs` drives the state machine with explicit timestamps, so hold, tap, double-tap and the 20-minute limit are tested without sleeping. `app.rs` runs whole dictations through a fake `Platform` and fake engines. The HTTP client, the model download and the Ollama calls run against a small local test server (`core/src/testing.rs`). The SQLite store runs in memory.
- **`src/`**: Vitest with jsdom. `tests/fakes.ts` provides a fake `api` and fake events, so each page and the bar are tested as DOM without Tauri.
- **`os/` and `src-tauri/`**: compiled and linted, not run in tests. They drive real devices and windows. Their logic is kept small on purpose. Only pure helpers (such as the key-name mapping in `os/src/keys.rs`) have tests.

## The coverage gate

`bash scripts/coverage.sh` requires:

| Part | Measured by | Requirement |
| --- | --- | --- |
| `core/` | `cargo llvm-cov` | Every line run by a test. The script parses the LCOV file and fails listing any line with zero hits |
| `src/` (TypeScript) | Vitest with V8 coverage | 100% of lines, functions, branches and statements (`vitest.config.ts`). `main.ts`, `bar.ts`, `api.ts` and `env.d.ts` are excluded: they only wire real dependencies |
| `os/`, `src-tauri/` | not measured | compiled and linted only |

`core/src/testing.rs` is excluded from the Rust measurement. Reports are written to `coverage/`.

You need `cargo-llvm-cov` 0.9.1 (`cargo install cargo-llvm-cov --version 0.9.1 --locked`) and the `llvm-tools-preview` component, which `rust-toolchain.toml` already lists.

A habit the code follows: a line that can only run when a test is about to fail (a `panic!` in a test helper) is written on one line with `#[rustfmt::skip]` so it is not counted as unexecuted code. See `text()` in `core/src/pipeline.rs`.

## CI

`.github/workflows/ci.yml` runs on pull requests and is reused by the release workflow on pushes.

- **Test (ubuntu-24.04, macos-15, windows-2025)**: fetch the runtime and the test model, then `npm ci`, `npm run build`, `npm test`, and `cargo test --workspace --locked`, and build the app with `cargo build -p speechify --locked`. On Linux only, it first runs `npm run format:check`, `cargo fmt --all --check` and `cargo clippy --workspace --all-targets --locked -- -D warnings`.
- **Tauri full coverage gate**: runs only after every Test job passed, then runs `scripts/coverage.sh` and uploads `coverage/` for 14 days.

A new pull request run cancels the one it supersedes. Release runs always finish.

## Performance and cleanup scenarios

On pull requests, `ci.yml` also measures performance. The jobs run in this order:

1. **Test (ubuntu-24.04, macos-15, windows-2025)** also builds the `perf` binary (`bench/src/bin/perf.rs`) in release mode and uploads it.
2. **Measure (linux, macos, windows)** starts once every Test job has finished. Each one takes the binary built on its own platform and runs `scripts/perf-run.sh`, which measures three things:
   - **Transcription**: speech to raw text on `jfk.wav` with the default speech model. It reports the load time, the median time to text over 3 warm runs, the speed relative to real time, and the share of words wrong.
   - **Cleanup**: raw text to the typed text. It reports the rule formatter's time over every scenario, and the language model's typical and slowest call times. It also counts how many of the model's answers the app rejected, using the rules instead.
   - **Scenarios**: each case in `bench/scenarios.json` runs through the rules alone and through the default language model, with the real system prompt (`core/src/prompts/cleanup.txt`), vetting and fallback. Results are grouped by `kind`: corrections, repetition, lists, paragraphs, fillers, formatting, and keep as said.

   The language model runs on the CPU under llama.cpp `b12211`, which `scripts/fetch-perf-llm.py` pins by SHA-256. CI gives each request 180 seconds instead of the app's 6, and sends short dictations to the model too, so the prompt is judged on its answers rather than on how fast the runner is.
3. **Performance report** waits for every Measure job and for the coverage gate. It merges the three platforms into one comment on the pull request, updated on every push, with a section each for Transcription, Cleanup and Scenarios. The comment:
   - compares every number with the latest results file on `main` and marks changes beyond ±15% (and beyond a small absolute floor);
   - lists scenarios that started or stopped passing;
   - shows every failing scenario, with what was said and what came out;
   - ends with a JSON summary for tools to read.

   Rows are named for what they measure, not for a model, so changing a default keeps them comparable. The footer names the models, and says when `main` was measured with different ones.

   When every Measure job and the coverage gate passed, it commits the merged results to the pull request branch as `bench/perf/results/pr-<number>.json`, authored by `github-actions[bot]`. Merging the pull request brings that file to `main`, and later pull requests compare with the most recently added results file there. Each pull request writes its own file, so two open pull requests never conflict over results.

None of this fails a pull request over a slower number or a worse scenario. The comment reports them.

### Default models

`core/defaults.json` holds the models the app starts with and the ones CI measures:

- `speech_model`: the speech model on a machine with a graphics card. It is also the model in `Settings::default()`, and the one Transcription measures.
- `cpu_speech_model`: what first start picks without a graphics card.
- `streaming_model`
- `cleanup_model`: the Ollama tag for cleanup, which Cleanup and Scenarios use.
- `teacher_speech_model` and `cpu_teacher_speech_model`: the nightly review's speech models, with and without a graphics card.
- `teacher_cleanup_model`: the nightly review's cleanup model.

Speech models are catalog ids (`core/src/catalog.rs`), and CI downloads them through the catalog with its pinned checksum. For the cleanup model, `scripts/fetch-perf-llm.py` reads the Ollama tag, looks it up in Ollama's registry, and downloads the same weights Ollama would, checked against the digest the registry gives. Ollama tags can move, so the results record that digest. Changing a default changes the app and the next CI measurement together. The catalog test fails if a speech default is not in the catalog.

After the results commit lands, pull before you push again: the branch now has a commit you do not have locally.

A commit pushed with the workflow token starts no workflows, and GitHub ignores checks from a `workflow_dispatch` run when it applies required checks. So once Performance report has pushed the results commit, it posts a passing commit status on it for each required check name (`Test (…)` and `Tauri full coverage gate`). It does this only after it has seen every Test job and the coverage gate pass on the commit it measured. The results commit adds one data file to that commit.

Only CI writes `bench/perf/results`. The coverage gate fails a pull request if any results file differs from the version `github-actions[bot]` last committed on that branch.

`release.yml` grants its `Tests` job the permissions that `ci.yml`'s jobs ask for, because a reusable workflow cannot ask for more than its caller. On pushes, Performance report, the only job that uses them, is skipped.

Scenario checks, all optional: `equals` (exact text), `contains` (each entry a phrase, or a list of alternatives), `absent` (phrases that must not remain, matched as whole words), `list` (`numbered`, `bullets` or `none`) with `items`, `paragraphs` (blocks separated by a blank line) and `min_lines`. `category` is `work` (the default, typing into Slack), `email`, `code` or `personal`. Some scenarios describe behaviour the app does not have yet, such as bullet points for items that are listed without being counted. They fail until it does, and the comment shows when that changes.

To run it locally (Linux):

```
bash scripts/fetch-test-fixtures.sh
python3 scripts/fetch-perf-llm.py .build/llm
cargo build --release --locked -p speechify-bench --bin perf
source .build/test-env.sh
bash scripts/perf-run.sh target/release/perf .build/llm .build/perf/linux.json
python3 bench/perf_report.py .build/perf --pr 0 --head-sha local --out-json .build/perf/record.json --out-md .build/perf/comment.md
```

Without `PERF_LLM_URL`, `target/release/perf bench/scenarios.json OUT.json` skips the cleanup model and runs everything else.

## Things to know

- Passing here is not the same as working on a person's desktop. The automated tests have passed on Linux, and the Windows and macOS test jobs run in CI, but nobody has used the app by hand on Windows or macOS yet. See the status section of the [README](../README.md).
- On a machine with a live Wayland session, do not launch the app to test it. It takes keys and pastes into whatever is focused. Use the tests.
- To check the formatter by hand without the app, the Style page has a "try it" box that runs `pipeline::compose` on text you type (command `try_format`).
