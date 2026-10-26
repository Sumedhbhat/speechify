# Tauri candidate testing and delivery

Use `desktop/tauri/README.md` for platform prerequisites, exact Mac/Windows build commands, permission setup, optional environments and the user device checklist. Use `candidate-parity.md` for every R01–R16 status and gap. The shared contract is `comparison-acceptance.md`.

## Development environment

- Ubuntu 26.04.1 LTS, Linux 7.0.0-38-generic, x86_64.
- AMD Ryzen 5 3500U, 4 cores / 8 logical CPUs, Radeon Vega Mobile graphics.
- Rust/cargo 1.99.0; Node 24.21.0; npm lockfile committed.
- System lacked compiler/pkg-config/development headers. `scripts/bootstrap-linux.py` extracted apt packages into the candidate's ignored `.build/sysroot`; no system package or service was changed. `scripts/local-env.sh` selects that toolchain.
- Desktop session was not entered for testing. No microphone, tray app, global keyboard source, autostart entry, remote Mac/Windows access or installed Speechify replacement was used.

## Reproduction

```sh
cd desktop/tauri
npm ci
python3 scripts/prepare-native.py
# On this local extracted-toolchain machine only:
source scripts/local-env.sh
npm run build
cd src-tauri
cargo test --locked --no-default-features
cargo check --locked
cd ..
npm run tauri -- build --ci --bundles deb -- --locked
python3 scripts/package-portable.py
python3 scripts/check-dependency-age.py
npm audit
```

The cutoff audit uses publication dates for every Cargo/npm lockfile entry. Its 600-package graph passed with zero entries newer than 2026-10-11. Direct pins and registry evidence are committed in `desktop/tauri/dependency-pins.json`; the complete audit is `dependency-age-evidence.json`. Optional Python lockfiles independently use the same cutoff and file hashes. No dependency overrides were introduced. npm audit reported zero vulnerabilities; a Rust advisory audit was not run.

## Named checks

- `audio::tests::{resampling,anti_alias}` verifies 48 kHz stereo conversion to 16 kHz mono and rejection of an above-Nyquist tone.
- `keys::tests::{taps_repeats_and_ctrl_c,combination}` verifies short taps, repeat suppression, combination matching and cancellation.
- `store::tests::{bounded_edits,no_guesses}` verifies word edits and rejection of self-generated training guesses.
- `store::persistence_tests::{restart_correction_export_and_safe_delete,consistent_import_copies_audio}` verifies persisted corrections/rules, labelled export, revert/delete/path guard and SQLite/audio import isolation.
- `models::tests::{checksum_and_cancel,catalog_is_complete}` verifies hash failure/cancellation and all 14 catalog entries (11 GGUF-backed).
- `worker::tests::cancellation_reaps_process_group` kills/reaps a test process group. This tests lifecycle plumbing, not native crash recovery on each platform.
- `learning::tests::{both_gates_required,hallucination_rejected}` requires both training gates and rejects repeated phrases.
- `config::tests::{config_validation,traversal}` validates configuration bounds and confined paths.
- Desktop-feature tests additionally cover clipboard text guard; Linux evdev mapping is a pure unit test and opens no device.

The early core run passed 15 tests. The final release/test evidence is recorded below and under `tauri-evidence/`. The frontend typecheck/production build and Linux desktop compilation passed. Build-only and simulated state tests are not OS acceptance evidence.

## Native fixture

- Runtime: transcribe.cpp **0.3.1**, Linux x86_64 wheel SHA-256 `9647bff7a7486810c2778fb9b86a6ac0302688620bfad8f31b560697423641aa`.
- Model: Moonshine base Q8_0, repository `handy-computer/moonshine-base-gguf`, revision `cb703342553d960a95e2b4b0f48b8142648ddb22`.
- Model SHA-256: `7f0027dfd857d310b63a85ef57cadf183da712cc374f85a648f8bc18aaa2efc8`, 77,476,480 bytes.
- Fixture: `https://raw.githubusercontent.com/ggml-org/whisper.cpp/master/samples/jfk.wav`; exact downloaded WAV digest and size recorded in final fixture evidence. Mono PCM16, 16 kHz, about 11 seconds.
- CPU device, four inference threads, English, one persistent native worker per command. No Python inference.
- First three development fixture calls returned real text: “And so my fellow Americans ask not what your country can do for you ask what you can do for your country”. Development inference times were 886.83, 1210.44 and 1184.87 ms. These are not the release comparison measurements.

For isolated release verification, set `SPEECHIFY_CANDIDATE_PARENT` to a test parent, install/copy the hash-validated model into its `speechify-tauri-candidate/models/moonshine-base.gguf`, then run:

```sh
/path/to/packaged/speechify-tauri-candidate fixture moonshine-base /absolute/path/jfk.wav 20 cpu
```

The command does not start Tauri, a tray, microphone or input source. Private fixture JSON includes actual text. Generic `logs/timings.jsonl` excludes it. Compare cold process wall time separately from `model_ready.load_ms` and each warm `inference_ms`; none of these is a cold filesystem-cache measurement. Release figures and exact artifact checksums are appended after packaging.

## Required user/lead verification

Mac and Windows runtime tests are explicitly user-deferred. The lead may perform serialized Linux device checks. Run at least 20 real dictations and 20 Settings open/close cycles per tested platform/session. Verify focus, newer clipboard copy, modifiers, short holds, disconnected inputs, model switch, offline restart, download cancellation, CPU fallback and no orphan children. Record actual windows/screenshots, idle CPU/RSS/PSS process-tree measurements and key→mic/release→paste timings. These are deliberately not replaced by fixture/UI mocks.

## Final Linux release evidence

The release-profile desktop test run passed **17 tests, zero failures** (`tauri-evidence/release-tests-final.log`), using `cargo test --release --locked`. The final Tauri Debian build succeeded (`release-build-delivery.log`). The real CLI model installer downloaded and SHA-256 verified all 77,476,480 bytes of Moonshine (`install-test.log`). Portable per-file checksums and the Debian archive inventory passed verification without installation.

The packaged portable executable completed **20 real CPU fixture transcriptions** with four threads and one persistent model load. Median inference was **732.785 ms**, p95 **872.185 ms**, native model load **124.729 ms**, and total process wall time **15,276.900 ms**. This was a fresh process with warm filesystem cache on a shared development host; it does not measure microphone, paste, UI, or cold-cache performance. Exact samples, text and hashes are in `tauri-evidence/fixture-release.json`.

Reproduce the packaged measurement from the repository root:

```sh
python3 desktop/tauri/scripts/verify-release.py \
  desktop/tauri/artifacts/speechify-tauri-candidate-linux-x86_64/speechify-tauri-candidate \
  desktop/tauri/.build/fixtures/jfk.wav \
  desktop/tauri/.build/fixtures/moonshine-base.gguf \
  --output /tmp/speechify-tauri-fixture.json \
  --parent /tmp/speechify-tauri-fixture-data
```

The fixture WAV digest is `59dfb9a4acb36fe2a2affc14bacbee2920ff435cb13cc314a08c13f66ba7860e` (352,078 bytes). Models and fixtures are external build inputs, not committed repository files.

Artifacts are local build outputs, not published releases. Paths below are relative to the worktree root; exact SHA-256 values are in `tauri-evidence/SHA256SUMS` (verify with `sha256sum -c docs/tauri-evidence/SHA256SUMS` from that root).

| Artifact | Bytes |
|---|---:|
| `desktop/tauri/src-tauri/target/release/speechify-tauri-candidate` | 19,185,176 |
| `desktop/tauri/src-tauri/target/release/bundle/deb/Speechify Tauri Candidate_0.1.0_amd64.deb` | 29,775,702 |
| `desktop/tauri/artifacts/speechify-tauri-candidate-linux-x86_64.tar.gz` | 29,190,538 |

The portable directory is `desktop/tauri/artifacts/speechify-tauri-candidate-linux-x86_64/`. Extract the archive, enter that directory, and run `./speechify-tauri-candidate` to launch the tray/settings application during authorized device testing. Running with the `fixture` subcommand only performs ASR. Packages exclude downloaded ASR models and optional Python environments. No macOS or Windows artifacts were built; native build/launch instructions, permissions and a manual checklist are in `desktop/tauri/README.md`.

## Independent lead review

The lead fixed full-dataset export silently inheriting the 1,000-row UI cap, and optional cleanup failures discarding valid local transcription. `cargo test --release --locked` passed 19 tests, including an older eligible recording beyond the history limit. Evidence: `tauri-evidence/lead-tests.txt`.

The packaged baseline window opened on an isolated Xvfb display with `GDK_BACKEND=x11`; `tauri-evidence/lead-window.png` shows its real initial missing-model state. This is window-rendering evidence only, not a microphone, focus, tray, hook or paste pass. The test app and virtual display subsequently terminated before interaction checks. No desktop acceptance pass is inferred.

The earlier Parakeet reference directory is no longer present; an interrupted partial candidate copy failed SHA-256 verification as intended. Matched comparison will use the still-available checksum-pinned Moonshine model and JFK fixture instead.

The corrected Linux Debian and portable builds completed successfully after commit `26157b2`. All portable manifest entries passed checksum verification. Updated distributable hashes are in `tauri-evidence/lead-SHA256SUMS`; earlier evidence describes earlier builds.
