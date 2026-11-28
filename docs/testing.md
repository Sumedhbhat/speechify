# Testing

## Commands

```
bash scripts/fetch-test-fixtures.sh   # speech runtime + two test models (46 MB and 90 MB), writes .build/test-env.sh
source .build/test-env.sh
cargo test --workspace --locked       # Rust tests
npm test                              # interface tests (Vitest, jsdom)
bash scripts/coverage.sh              # the coverage gate: core, controller and ui
bash scripts/coverage.sh controller   # one part of it
npx moon run :lint :test :build --affected --downstream deep   # only what your uncommitted changes reach
```

On Windows run the script under Git Bash and pass `python`: `bash scripts/fetch-test-fixtures.sh python`.

`fetch-test-fixtures.sh` runs `fetch-native.py`, then downloads two models pinned by revision and SHA-256 and cached in `.build/models`: Whisper tiny.en (Q8_0, 46 MB) and Parakeet TDT/CTC 110M (Q4_K_M, 90 MB, the only small model that reports per-word confidence). It writes `.build/test-env.sh`, which sets `SPEECHIFY_NATIVE_DIR`, `SPEECHIFY_TEST_MODEL` and `SPEECHIFY_TEST_TOKEN_MODEL`. Rerunning it is cheap: files that match their checksums are not downloaded again.

The speech-engine tests in `core/src/native.rs` run against the real runtime and those models, using `core/tests/fixtures/jfk.wav`. Without the variables they skip, unless `SPEECHIFY_REQUIRE_NATIVE=1` is set, in which case they fail. CI and `coverage.sh` set it, so a missing runtime cannot pass quietly.

Useful narrower runs:

```
cargo test -p speechify-core format        # the formatter
cargo test -p speechify-controller session
npx vitest run --root ui tests/format.test.ts
npx vitest                                 # watch mode
```

## What is tested, and how

- **`core/`**: unit tests sit beside the code. The formatter passes are tables of input and output pairs. The HTTP client, the model download and the Ollama calls run against a small local test server (`core/src/testing.rs`). The SQLite store runs in memory.
- **`controller/`**: `session.rs` drives the state machine with explicit timestamps, so hold, tap, double-tap and the 20-minute limit are tested without sleeping. `app.rs` runs whole dictations through a fake `Platform` and the fake engines that `core` exports behind its `fakes` feature.
- **`ui/`**: Vitest with jsdom. `ui/tests/fakes.ts` provides a fake `api` and fake events, so each page and the bar are tested as DOM without Tauri.
- **`os/` and `src-tauri/`**: compiled and linted, not run in tests. They drive real devices and windows. Their logic is kept small on purpose. Only pure helpers (such as the key-name mapping in `os/src/keys.rs`) have tests.

## The coverage gate

`bash scripts/coverage.sh` requires:

| Part | Measured by | Requirement |
| --- | --- | --- |
| `core/` | `cargo llvm-cov` | Every line run by a test. The script parses the LCOV file and fails listing any line with zero hits |
| `controller/` | `cargo llvm-cov` | The same, counting only `controller/` lines |
| `ui/src/` (TypeScript) | Vitest with V8 coverage | 100% of lines, functions, branches and statements (`ui/vitest.config.ts`). `main.ts`, `bar.ts`, `api.ts` and `env.d.ts` are excluded: they only wire real dependencies |
| `os/`, `src-tauri/` | not measured | compiled and linted only |

`core/src/testing.rs` is excluded from the Rust measurement. Reports are written to `coverage/`.

You need `cargo-llvm-cov` 0.9.1 (`cargo install cargo-llvm-cov --version 0.9.1 --locked`) and the `llvm-tools-preview` component, which `rust-toolchain.toml` already lists.

A habit the code follows: a line that can only run when a test is about to fail (a `panic!` in a test helper) is written on one line with `#[rustfmt::skip]` so it is not counted as unexecuted code. See `text()` in `core/src/pipeline.rs`.

## CI

`.github/workflows/ci.yml` runs on pull requests and is reused by the release workflow on pushes.

moon decides what each run checks. The first step of every Test job, `tools/ci/plan.mjs`, runs `moon query affected <base>:HEAD --downstream deep` to find which tasks the pull request reaches, and writes the list and the reason for each task to the job summary. Every later step runs only those tasks, with `npx moon run`. Pushes to `main` and release branches run every task. A pull request that only edits `docs/` or top-level Markdown runs none; one that changes `.moon/`, `tools/ci/` or `ci.yml` runs all of them. See [architecture.md](architecture.md#projects-and-their-gates) for which change reaches which task.

- **Test (ubuntu-24.04, macos-15, windows-2025)**: plans, fetches the runtime and test models only if `core:test` will run, then runs the affected `test` and `build` tasks. On Linux only, it first runs `tools/ci/check-graph.mjs`, which fails if a `dependsOn` in a `moon.yml` no longer matches the crate's path dependencies in `Cargo.toml`, then the affected `lint` and `test-report` tasks: `cargo fmt` and `cargo clippy -D warnings` per crate, Prettier for `ui`, and the report's Python tests. A pull request that changes nothing in a project, such as one that only edits `docs/`, passes without building anything.
- **Tauri full coverage gate**: runs only after every Test job passed, then runs the affected `coverage` tasks (`scripts/coverage.sh core`, `controller`, and the `ui` Vitest run) and uploads `coverage/` for 14 days.

These three job names are required checks, so they always run; a job with nothing to do passes.

A new pull request run cancels the one it supersedes. Release runs always finish.

## Performance and cleanup scenarios

On pull requests and on pushes to `main`, `ci.yml` also measures performance, but only when the moon task `perf:measure` is affected. That happens when `core/` or `perf/` changes, or the toolchain, the speech runtime pin, the report action, or the CI and moon setup. A change to the interface, the desktop shell, the controller, `os/`, `bench/` or the docs does not run it, and neither does a `Cargo.lock`-only bump. On `main` the comparison is with the commit the newest nightly's `performance.json` was measured at, so a cancelled or failed measurement is made up on the next merge. If that file or commit can't be found, the gate runs. Release branches never run it. The Test job's summary says why the gate ran or why it was skipped. When it is skipped there are no Measure jobs and no comment. The nightly release then gets the previous nightly's `performance.json`, copied unchanged, so it still names the commit it was measured at.

The jobs run in this order:

1. **Test (ubuntu-24.04, macos-15, windows-2025)** also builds the `perf` binary (`perf/src/main.rs`, `moon run perf:release`) in release mode and uploads it.
2. **Measure (linux, macos, windows)** starts as soon as the Test job of its own platform passes, without waiting for the other platforms. Each platform is its own pair of jobs (`test-linux` then `measure-linux`, and so on) rather than one matrix, because a matrix job waits for every entry of the matrix before the next job starts. The pairs share their steps through YAML anchors. Each Measure job takes the binary built on its own platform and runs `perf/run.sh`, which measures three things:
   - **Transcription**: speech to raw text on `jfk.wav` with the default speech model. It reports the load time, the median time to text over 3 warm runs, the speed relative to real time, and the share of words wrong.
   - **Cleanup**: raw text to the typed text. It reports the rule formatter's time over every scenario, and the language model's typical and slowest call times. It also counts how many of the model's answers the app rejected, using the rules instead.
   - **Scenarios**: each case in `perf/scenarios.json` runs through the rules alone and through the default language model, with the real system prompt (`core/src/prompts/cleanup.txt`), vetting and fallback. Results are grouped by `kind`: corrections, repetition, lists, paragraphs, fillers, formatting, and keep as said.

   The language model runs under Ollama `v0.40.19`, which `perf/fetch-ollama.py` pins by SHA-256, and is called through Ollama's own `/api/chat` with thinking off, the way the app calls it by default. `perf/run.sh` starts an Ollama of its own on port 8089 (`PERF_LLM_PORT`) with its models in `.build/llm/models`, one request at a time and a 4096-token context. On Linux and Windows the fetch keeps only Ollama's CPU libraries, leaving out the CUDA and Vulkan ones, so the model runs on the CPU; on macOS Ollama uses Metal when the machine offers it, as the app does. CI gives each request 180 seconds instead of the app's 6, and sends short dictations to the model too, so the prompt is judged on its answers rather than on how fast the runner is.
3. **Performance report** waits for every Measure job and runs `.github/actions/performance-report` with one input, `target`:
   - `branch` (pull requests): downloads `performance.json` from the newest nightly release that has one, and adds a new comment to the pull request on every run, headed with the commit it measured, so the comments read as a timeline of what each push changed. The layout is the template `perf/perf_comment.md`; `perf_report.py` fills in its `$name` fields, so a wording or order change goes in the template, not the script.
   - `main` (pushes to `main`): saves the three platforms as the `performance-baseline` artifact, only when all three were measured. `release.yml` attaches it to the nightly release as `performance.json`. When there is none, because the gate was skipped or a platform was missing, `release.yml` copies `performance.json` from the newest earlier nightly that has one.

   The comment:
   - compares every number with that nightly release and marks changes beyond ±15% (and beyond a small absolute floor);
   - lists scenarios that started or stopped passing;
   - shows every failing scenario, with what was said and what came out;
   - ends with a JSON summary for tools to read, keyed by stable metric names such as `time_ms` and `llm_p90_ms`.

   The comment shows at most 40 failing rows, with long text shortened, so it stays under GitHub's size limit. The run's job summary has everything.

   Rows are named for what they measure, not for a model, so changing a default keeps them comparable. The footer names the models, and says when `main` was measured with different ones. Results carry a format number, `SCHEMA` in `perf.rs` and in `perf_report.py`, and a test checks that the two match. Bump both when the results JSON changes shape. Until a nightly has results in the new format, pull requests say the baseline is in an older format instead of comparing.

   Nothing is committed. The results live only on releases, so CI never pushes to a pull request branch and never starts runs that wait for approval. A merge with `[no-release]` publishes no nightly, so the next pull request compares with the release before it.

None of this fails a pull request over a slower number or a worse scenario. The comment reports them. On `main`, Measure and Performance report may fail without failing the release.

### Default models

`core/defaults.json` holds the models the app starts with and the ones CI measures:

- `speech_model`: the speech model on a machine with a graphics card. It is also the model in `Settings::default()`, and the one Transcription measures.
- `cpu_speech_model`: what first start picks without a graphics card.
- `streaming_model`
- `cleanup_model`: the Ollama tag for cleanup, which Cleanup and Scenarios use.
- `teacher_speech_model` and `cpu_teacher_speech_model`: the nightly review's speech models, with and without a graphics card.
- `teacher_cleanup_model`: the nightly review's cleanup model.

Speech models are catalog ids (`core/src/catalog.rs`), and CI downloads them through the catalog with its pinned checksum. For the cleanup model, `perf/run.sh` pulls the Ollama tag into its own Ollama, which checks the download against the registry's digests. Ollama tags can move, so the results record the digest of the weights that were pulled (`perf/fetch-ollama.py digest`). Changing a default changes the app and the next CI measurement together. The catalog test fails if a speech default is not in the catalog.

`release.yml` grants its `Tests` job `pull-requests: write` because Performance report asks for it, and a reusable workflow cannot ask for more than its caller. On pushes it does not comment.

Each scenario needs an `id`, a `kind`, the `raw` text and at least one check. `perf` refuses to run on an unknown key, a duplicate id, `items` without `list`, or an unknown category, so a misspelt check cannot pass silently. Scenario checks: `equals` (exact text), `contains` (each entry a phrase, or a list of alternatives), `absent` (phrases that must not remain, matched as whole words), `list` (`numbered`, `bullets` or `none`) with `items`, `paragraphs` (blocks separated by a blank line) and `min_lines`. `category` is `work` (the default, typing into Slack), `email`, `code` or `personal`. Some scenarios describe behaviour the app does not have yet, such as bullet points for items that are listed without being counted. They fail until it does, and the comment shows when that changes.

To run it locally (Linux or macOS):

```
npx moon run perf:measure          # perf/measure.sh: fetches, builds and writes .build/perf/local.json
python3 perf/perf_report.py .build/perf --head-sha local --out-json .build/perf/record.json --out-md .build/perf/comment.md
```

`perf/measure.sh` runs `fetch-test-fixtures.sh`, `perf/fetch-ollama.py .build/llm`, the release build, and `perf/run.sh`, which needs port 8089 free; set `PERF_LLM_PORT` to use another. It never uses an Ollama you already run. Without `PERF_LLM_URL`, `target/release/perf perf/scenarios.json OUT.json` skips the cleanup model and runs everything else.

Tests: `npx moon run perf:test` (`cargo test -p speechify-perf`) covers the scenario checker and validation, and `npx moon run perf:test-report` (`python3 -m unittest discover -s perf -p 'test_*.py'`) covers the report. CI runs the first on every platform and the second on Linux, whenever `perf/` is affected.

## Things to know

- Passing here is not the same as working on a person's desktop. The automated tests have passed on Linux, and the Windows and macOS test jobs run in CI, but nobody has used the app by hand on Windows or macOS yet. See the status section of the [README](../README.md).
- On a machine with a live Wayland session, do not launch the app to test it. It takes keys and pastes into whatever is focused. Use the tests.
- To check the formatter by hand without the app, the Style page has a "try it" box that runs `pipeline::compose` on text you type (command `try_format`).
