# Behavioral tests

For a local refactor check, install the formatting tools with
`python -m pip install -r requirements-dev.txt` and run `npm run check`.
Use `npm run format` to apply Prettier, Ruff and rustfmt. The native coverage
matrix also checks formatting before collecting coverage.

The TypeScript tests use rendered controls and the Tauri command boundary. They
assert visible text, saved form values, navigation, and requests received by the
Rust service. Run them with `npm run test:coverage`. Every application TypeScript
file is included, with 100% thresholds for statements, lines, functions and
branches.

The Rust tests exercise the command service, persisted configuration, SQLite
history, corrections, model installation, worker processes, and training
adoption. Native-engine and trainer fixtures are separate compiled programs.
They let tests verify the real ABI and process protocols with small deterministic
inputs. Model downloads use a local HTTP server and real SHA-256 verification.
Actual speech recognition can also be checked with the application's `fixture`
command and an installed native model.
The coverage runner caches the catalog's pinned Moonshine GGUF (about 77 MB)
under `.build/fixtures` for the real model-reuse and disk-failure scenarios.
It verifies the catalog SHA-256 before using this artifact.

Every pull request runs the native coverage matrix, including pull requests
that change files outside Tauri. The `Tauri full coverage gate` check is required
by the repository's `main: require CI` ruleset. It runs even when a matrix job
fails, and fails if any platform's behavioral suite fails or is cancelled or
skipped. Only successful suites proceed to the combined 100% coverage check.

The scenario suites run in CI as follows:

| Behavior | Suites | CI execution |
| --- | --- | --- |
| Rendered controls, navigation, forms, history, models and learning | `tests/interface.test.ts` | All three native coverage jobs via `npm run test:coverage` |
| Settings, persisted history, corrections, audio retention, cleanup and cancellation | `src-tauri/tests/unit/{behavior,service}.rs` and source unit tests | All three native coverage jobs |
| Native ABI, model loading, downloads, verification, optional installation and recovery | `src-tauri/tests/unit/{native,models,installer}.rs` | All three native coverage jobs |
| Training, teacher selection, adoption, rejection, rollback and scheduling | `src-tauri/tests/unit/learning.rs` | All three native coverage jobs; controlled delayed-IO races on Linux |
| Microphone samples, recording, hotkeys, manual actions and disconnect recovery | `src-tauri/tests/unit/{capture,desktop,evdev}.rs` | Portable assertions on all platforms; synthetic ALSA, X11 and Wayland recordings on Linux |
| Clipboard preservation, newer copies, focus and correction safeguards | `src-tauri/tests/unit/{platform,clipboard,windows,macos,xclipboard}.rs` | Native Windows/macOS desktops and isolated Linux X11 session |
| External CLI, worker protocol, cancellation, startup and shutdown | `tests/cli.test.py` | All three native coverage jobs; Unix watchdog and Linux clipboard/desktop cases run on their supported hosts |
| Coverage rejection and collector correctness | `tests/coverage-{gate,runtime}.test.py` | All three native coverage jobs; Linux continuous profiling checked on Linux |
| Optional Python environment installer interruption and recovery | `scripts/test-optional-installer.py` | All three native coverage jobs and Linux/macOS package jobs |

The Rust coverage command includes ignored tests, and CI enables the disposable
desktop and virtual keyboard fixtures. OS-specific cases run on their native
host. The ordinary package test commands can omit those fixtures because the
coverage matrix runs them explicitly.

Install `cargo-llvm-cov` 0.9.1, `rustfilt` 0.2.0, and the Rust
`llvm-tools-preview` component, then run:

```sh
npm ci
bash scripts/test-coverage.sh
```

The script prepares the pinned native runtime, builds the frontend, runs the
full desktop tests, and drives the built CLI through its stdin/stdout protocol.
Linux needs Xvfb, xauth, xclip, xdotool, D-Bus and gdbus in addition to the desktop
build prerequisites. Its desktop tests use private display and D-Bus sessions
and a synthetic audio device. The coverage runner compiles an ALSA fixture
with the existing C compiler and ALSA development headers. Controlled IO
responses reproduce cancellation after a worker reply and after inference has
finished, while the production worker and recorder protocols remain active. This driver supplies
float, signed and unsigned samples, setup/start failures, and disconnects;
it never opens microphone hardware. macOS/Windows clipboard tests are opt-in because
they modify the system clipboard: the CI job runs them on disposable runners
with `SPEECHIFY_TEST_DISPOSABLE_DESKTOP=1` and `--include-ignored`.
The macOS adapter scenarios use an isolated child process with controlled
Quartz permission, focus and key-state responses. Paste events are observed at
the OS boundary, while clipboard snapshots use the real native pasteboard.
Windows error scenarios replace individual imported OS functions in an isolated
test child, then restore them before checking that clipboard and worker startup
can recover. macOS uses native objects with controlled pasteboard responses.
The Linux CI runner also creates a private, empty `/dev/input/by-id` directory;
a FIFO in that directory exercises the real Wayland keyboard adapter and
recording cancellation on disconnect. Local runs omit that scenario unless
`SPEECHIFY_TEST_VIRTUAL_KEYBOARD=1` is explicitly set with the same isolated
fixture available. The full gate still requires those paths to be covered.
It writes reports under `coverage/` and runs the coverage gate. Use
`--collect-only` to collect an individual platform's report before combining it
with the other platforms.

The CI gate combines native Linux, macOS and Windows LCOV reports. It requires
100% Rust line and function coverage across all files in `src-tauri/src`, plus
the frontend thresholds. Platform-specific files stay in the inventory even
when the current OS cannot compile them. Linker symbols are demangled before
combining execution counts from different binaries and target triples.
Generic instantiations count as one source function; distinct closure bodies
remain separate. Source and dependency fingerprints prevent combining reports
from different versions of the application.
LLVM source spans keep distinct closures separate, including closures on the
same line. They also match the same body when conditional compilation changes
its closure ordinal on another OS. Source locations keep same-named platform
implementations separate, so running
the X11 poller cannot satisfy the Windows or macOS poller's function coverage.
LLVM can attribute Tauri SDK serialization closures to the closing bracket of
`#[tauri::command]`. The gate omits that non-executable annotation; the authored
command handler and its error handling remain in the coverage inventory.
Linux uses continuous LLVM profiling so pre-exec child setup and cancelled
helpers retain their execution counts. A standalone collector test verifies
this behavior before the application run. Add `--details` to the gate command
to list the uncovered lines and functions.

```sh
python3 scripts/check-coverage.py \
  --frontend coverage/coverage-final.json \
  --backend linux/rust.info macos/rust.info windows/rust.info
```

Tests and their fixtures live outside application source directories. Build
scripts, generated type declarations, and the legacy Python desktop application
are outside this Rust/TypeScript coverage gate. The legacy application is still
built and tested by the repository CI and release workflows, so removing it
requires a separate release migration. Tauri's optional Python model
and training resources remain runtime dependencies; the existing optional
installer tests run with `python3 scripts/test-optional-installer.py`.

The test dependency lock uses releases available by 2026-10-11. Undici 8.12.3 is
the security exception: versions below 8.12.2 have the advisories reported by
`npm audit`, including GHSA-xjj6-f33q-929c and GHSA-2m5v-pch6-74hx. The exception
uses the dependency's supported version range and introduces no override.
