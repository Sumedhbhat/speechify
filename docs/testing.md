# Testing

## Commands

```
bash scripts/fetch-test-fixtures.sh   # once: speech runtime + a 46 MB test model, writes .build/test-env.sh
source .build/test-env.sh
cargo test --workspace --locked       # Rust tests
npm test                              # interface tests (Vitest, jsdom)
bash scripts/coverage.sh              # the coverage gate
```

On Windows run the script under Git Bash and pass `python`: `bash scripts/fetch-test-fixtures.sh python`.

`fetch-test-fixtures.sh` runs `fetch-native.py`, then downloads Whisper tiny.en (Q8_0, pinned by revision and SHA-256, cached in `.build/models`) and writes `.build/test-env.sh`. That file sets `SPEECHIFY_NATIVE_DIR` and `SPEECHIFY_TEST_MODEL`.

The speech-engine tests in `core/src/native.rs` run against the real runtime and that model, using `core/tests/fixtures/jfk.wav`. Without the two variables they skip, unless `SPEECHIFY_REQUIRE_NATIVE=1` is set, in which case they fail. CI and `coverage.sh` set it, so a missing runtime cannot pass quietly.

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

## Things to know

- Passing here is not the same as working on a person's desktop. The automated tests have passed on Linux, and the Windows and macOS test jobs run in CI, but nobody has used the app by hand on Windows or macOS yet. See the status section of the [README](../README.md).
- On a machine with a live Wayland session, do not launch the app to test it. It takes keys and pastes into whatever is focused. Use the tests.
- To check the formatter by hand without the app, the Style page has a "try it" box that runs `pipeline::compose` on text you type (command `try_format`).
