# Behavioral tests

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

Install `cargo-llvm-cov` 0.9.1, `rustfilt` 0.2.0, and the Rust
`llvm-tools-preview` component, then run:

```sh
npm ci
bash scripts/test-coverage.sh
```

The script prepares the pinned native runtime, builds the frontend, runs the
full desktop tests, and drives the built CLI through its stdin/stdout protocol.
It writes reports under `coverage/` and runs the coverage gate. Use
`--collect-only` to collect an individual platform's report before combining it
with the other platforms.

The CI gate combines native Linux, macOS and Windows LCOV reports. It requires
100% Rust line and function coverage across all files in `src-tauri/src`, plus
the frontend thresholds. Platform-specific files stay in the inventory even
when the current OS cannot compile them. Linker symbols are demangled before
combining execution counts from different binaries and target triples.

```sh
python3 scripts/check-coverage.py \
  --frontend coverage/coverage-final.json \
  --backend linux/rust.info macos/rust.info windows/rust.info
```

Tests and their fixtures live outside application source directories. Build
scripts, generated type declarations, and the legacy Python desktop application
are outside this Rust/TypeScript coverage gate. Tauri's optional Python model
and training resources remain runtime dependencies; the existing optional
installer tests run with `python3 scripts/test-optional-installer.py`.

The test dependency lock uses releases available by 2026-10-11. Undici 8.12.3 is
the security exception: versions below 8.12.2 have the advisories reported by
`npm audit`, including GHSA-xjj6-f33q-929c and GHSA-2m5v-pch6-74hx. The exception
uses the dependency's supported version range and introduces no override.
