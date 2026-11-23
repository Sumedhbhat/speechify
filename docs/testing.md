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

`.github/workflows/performance.yml` runs on every pull request. It never fails a pull request over a slower number or a worse scenario. It reports them in a comment.

- **Measure (linux, macos, windows)**: builds `speechify-bench`'s `perf` binary in release mode on `ubuntu-24.04`, `macos-15` and `windows-2025`, then runs `scripts/perf-run.sh`. That times the whole dictation pipeline (voice detector, recognition, formatting) on `jfk.wav` with the two test models (load time, then the median of 5 warm runs), times the rule formatter over every scenario, and runs each scenario in `bench/scenarios.json` twice: through the rules alone, and through the cleanup model with the real system prompt (`core/src/prompts/cleanup.txt`), vetting and fallback. The cleanup model is gemma-3-4b-it Q4_K_M (the app's default `gemma3:4b`), served on the CPU by llama.cpp `b12211`. Both are pinned by SHA-256 in `scripts/fetch-perf-llm.py`. CI uses a 180-second request budget instead of the app's 6 seconds, so the prompt is judged on its answers rather than on runner speed.
- **Performance report**: merges the three results, compares them with the latest results file on `main`, and posts one comment on the pull request, updated on every push. The comment marks timing changes beyond ±15% (and beyond a small absolute floor), lists scenarios that started or stopped passing, shows every failing scenario with what was said and what came out, and ends with a JSON summary for tools to read.
- **Record performance results**: waits for `Tauri full coverage gate` to pass on the same commit, then commits the merged results to the pull request branch as `bench/perf/results/pr-<number>.json`, authored by `github-actions[bot]`. Merging the pull request brings that file to `main`, and the most recently added results file on `main` is what later pull requests compare with. Each pull request writes its own file, so two open pull requests never conflict over results.

After the results commit lands, pull before you push again: the branch has a commit you do not have locally.

A commit pushed with the workflow token starts no workflows, so the results commit would have no required checks. `Record performance results` starts `.github/workflows/ci-results.yml` for it. That workflow's jobs have the required check names. Each one passes only if the head commit is from `github-actions[bot]`, changes nothing but `bench/perf/results/*.json`, and the same check passed on the commit before it. GitHub only starts `workflow_dispatch` workflows that already exist on `main`, so this works from the first pull request after the one that added it.

Only CI writes `bench/perf/results`. `Tauri full coverage gate` fails a pull request with a commit touching that folder unless `github-actions[bot]` wrote it.

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
