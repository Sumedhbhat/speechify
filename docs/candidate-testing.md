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


## Implementation continuation (2026-10-27)

Continuation started from lead evidence commit `a38e6da` on the same `sb/tauri-candidate-implementation` worktree. No reset, new checkout, PR, CI workflow, push, release, remote access or installed-app/system-policy change was made. The original pinned fixtures remain byte-for-byte intact. The host is still Ubuntu 26.04.1, kernel 7.0.0-38, Ryzen 5 3500U (4 cores/8 threads), Rust 1.99.0 and Node 24.21.0. The worktree now resolves through the `/workspaces/bhatsumedh/` mount; `/home/bhatsumedh/` is an alias.

`followup-release-tests.log` records **25 passing release tests** with desktop features. Added regressions exercise retention-off audio recovery/eviction/restart, saving a correction even when replacement is refused, manual-recording short-tap/cancel command routing without opening a microphone, invalid-install recovery, worker reload settings, teacher memory/CPU consent, UTF-16 replacement ranges and supported X11 snapshot formats. Both lead regressions still pass.

`followup-optional-tests.log` records **4 passing offline installer tests**: interrupted partial recovery, hash mismatch and retry, dependency failure without completion marker, and oversized file rejection. Subprocess/network operations are substituted only in these installer tests; they do not prove optional inference or real multi-gigabyte installation.

`followup-adapter-typecheck.log` records Windows and Mac adapter Rust metadata type checks on Linux, with only cfg/link annotations adapted. This catches Rust type errors but **does not validate native linking, ABI correctness, permissions or runtime behavior**. Windows child Job Object startup still requires native compilation. The lead will add a branch-only native Mac packaging workflow after handoff; this implementation does not add CI or push. Native resource preparation uses pinned architecture-specific wheels and retains the existing app/dmg configuration.

Reproduce these additional checks from the worktree root:

```sh
source desktop/tauri/scripts/local-env.sh
cargo test --release --locked --manifest-path desktop/tauri/src-tauri/Cargo.toml
python3 desktop/tauri/scripts/test-optional-installer.py
python3 desktop/tauri/scripts/check-platform-adapters.py
```

`followup-frontend-build.log` records TypeScript checking and Vite production output. `followup-linux-build.log` records the actual Debian rebuild. Current artifact and fixture results are recorded in the follow-up manifests below; earlier baseline and lead logs/hashes are preserved as historical evidence and refer to their earlier binaries.

New manual acceptance cases for the lead/user:

- X11: copy identical text again during restoration; test an owner copying again during snapshot. Verify XFixes generation refusal, HTML/RTF/small PNG restoration, unsupported-format/large-data refusal, and failure with missing xclip/timeout/XFixes. No INCR or clipboard-manager-on-quit support is claimed.
- Mac: test multiple pasteboard item/type snapshots, same-text changeCount protection, Accessibility focus guard and explicit save-only correction behavior. AppKit's count-check/write race is documented in parity limits.
- Windows: test focused child-control changes within the same top-level window, clipboard sequence protection, handle-format refusal, Job Object cleanup after parent crash, and standard Edit replacement with unchanged/changed text, caret, document and expired receipt. Browser/rich-edit targets must save only. Win32 cross-process edit messages are not atomic against concurrent target mutation.
- Wayland: Record in app → Stop and transcribe → Copy text without evdev grants; global hold reports its explicit evdev requirement. Automatic target/paste is unavailable and no elevation occurs.
- Optional models: explicitly install Python 3/uv, use Models/Learning progress actions, cancel during dependency/model stages, retry offline/network/disk failure, and ensure only complete model directories are selectable. Training/inference remain native-runtime acceptance work.


### Final continuation packages and fixture evidence

The final Linux package executable completed **20/20 real fixture transcriptions** with English, CPU, four threads and exactly **one persistent model load**. Median inference: **718.034 ms**; p95: **742.434 ms**; native load: **118.300 ms**; process wall time: **14745.361 ms**. These are shared-host, warm-filesystem measurements, not desktop latency. Exact samples and hashes: `tauri-evidence/followup-fixture-release.json`.

All **54 portable manifest entries** pass. The extracted Debian executable matches the measured portable binary, the optional installer matches current source, and the native library matches the portable resources. Both packages exclude Python bytecode caches. The no-display clipboard-helper command refuses cleanly without starting desktop services. Evidence: `followup-package-verification.json`, `followup-portable-verify.log`, `followup-debian-inventory.txt` and `followup-headless-clipboard.json`.

After `tauri build`, the Debian archive was regenerated with `npm run tauri -- bundle --ci --bundles deb --no-binary-patching` so both distributions contain the exact measured binary rather than differing in Tauri updater bundle metadata. No updater is configured. This supported bundle step is recorded in `followup-resource-rebundle.log`.

Current artifacts (relative to worktree root):

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| `desktop/tauri/src-tauri/target/release/speechify-tauri-candidate` | 19,295,768 | `e97225a416d911fdd9a06415f2acfbff6bf227563800f0f182cf9dcc20f2ceea` |
| `desktop/tauri/src-tauri/target/release/bundle/deb/Speechify Tauri Candidate_0.1.0_amd64.deb` | 29,825,800 | `53a6eb8215e33b3d0d3fad70e29512156cab7c90d240949215f37f77355cd236` |
| `desktop/tauri/artifacts/speechify-tauri-candidate-linux-x86_64.tar.gz` | 29,239,288 | `f242dda83d8e17590ba6db193f911dd258491a9f82aef7a8568d5a7a7d0b1f64` |

Verify current outputs with `sha256sum -c docs/tauri-evidence/followup-SHA256SUMS`. Baseline and lead manifests are retained as historical records. There is no claim of full parity or post-change desktop acceptance: the explicit remaining platform/runtime blockers are in `candidate-parity.md`.


## Lead Mac packaging verification — 2026-10-27

Apple Silicon macOS candidate packaging passed on the native macos-15 GitHub runner. Compilation, tests, ad-hoc app signing verification, DMG creation and checksum verification passed. This does not establish microphone, focus, paste, permissions or tray runtime correctness. Mac and Windows runtime testing remains user-deferred. No release was published.

Build evidence: https://github.com/Sumedhbhat/speechify/actions/runs/40442173851

The authoritative comparison, downloads and unresolved acceptance gaps are at https://bhatsumedh.tail429e81.ts.net/pages/speechify-candidate-comparison/ . These personal test DMGs are not Developer-ID notarized. Use Finder and the app-specific Open Anyway control if needed; grant permissions to each distinct candidate.
