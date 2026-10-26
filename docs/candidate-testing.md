# Electron build, verification and device handoff

The implementation starts at `c18ba51068186e76052cc99a8b54edcf00c4270c`, in branch `sb/electron-desktop-candidate`, worktree `/home/bhatsumedh/workspaces/worktrees/speechify/t3-dfc8c477`. The final commit SHA is supplied with the handoff. The candidate has known parity gaps; consult `candidate-parity.md` before device testing.

## Build locally

Prerequisites: Node 24.21+ and npm, network for initial dependency/native-resource installation, and the OS-specific tools below. Native libraries are architecture-specific; build on the target architecture. The normal application needs no Python.

```sh
cd desktop/electron
npm ci
node node_modules/electron/install.js
npm run native
npm test
npm run package
```

`npm run native` obtains the standard transcribe.cpp **0.3.1** wheel as an archive, verifies the baseline SHA-256, extracts only native libraries and licenses, and records `resources/native/provenance.json`. It does not install or invoke Python. `npm ci` uses the committed lockfile and approved native dependency install scripts. If a package manager blocks approved scripts, inspect `package.json`'s `allowScripts`; do not disable script review globally.

Output stays in `desktop/electron/artifacts/`. The package command never installs, launches a tray, opens a microphone, enables startup, or changes the existing Speechify application. `artifacts/SHA256SUMS` covers distributable files. The unpacked package excludes model weights. Use Models → **Use existing file…** to reference a pinned GGUF read-only, or Install to download into candidate-owned storage.

## Linux x64

Built on Ubuntu 26.04.1 LTS, x86_64, AMD Ryzen 5 3500U with Radeon Vega Mobile Gfx, Node 24.21.0 build host. The packaged Electron runtime reports Node 24.22.0. Required packaging tools: `unzip`, `tar`, `dpkg-deb`. Runtime needs GTK3, NSS, ALSA, X11/xtst libraries; X11 paste additionally needs `xdotool`. The `.deb` declares runtime dependencies. Vulkan needs a working system Vulkan driver; CPU remains available.

Artifacts:

- `desktop/electron/artifacts/speechify-electron-candidate-linux-x64/` — unpacked application.
- `desktop/electron/artifacts/speechify-electron-candidate-linux-x64.tar.gz` — portable package.
- `desktop/electron/artifacts/speechify-electron-candidate-linux-x64.deb` — isolated Debian package, not installed during implementation.
- `desktop/electron/artifacts/SHA256SUMS` — checksum manifest.

Launch after coordinating device testing:

```sh
./artifacts/speechify-electron-candidate-linux-x64/speechify-electron-candidate
```

The portable build requires usable unprivileged user namespaces for Chromium sandboxing. On systems restricting them, use the `.deb` with normal explicit package installation; it owns a root-owned setuid Chromium sandbox helper. The build does not silently change system policy and does not default to `--no-sandbox`. No package installation was performed here.

Wayland uses the same Linux package but has **no implemented global hold-key or focus-safe paste integration**. Manual recording and Copy text are available. Do not infer a Wayland pass from an X11 result.

## macOS: user-run build and launch

No Mac artifact was built on Linux. Use a local Mac with macOS 14+ and Node 24, Apple Command Line Tools (`xcode-select --install` if absent), and matching arm64 or x64 Node. `unzip`, `codesign`, `hdiutil`, and `iconutil` where desired come from macOS/tooling. The script supports the baseline native wheel for Apple Silicon or Intel and rejects resource/host architecture mismatches.

Run the common build commands above **on the Mac**. `npm run package` produces:

```text
artifacts/speechify-electron-candidate-darwin-arm64/Speechify Electron Candidate.app
artifacts/speechify-electron-candidate-mac-arm64.dmg
```

For Intel, replace `arm64` with `x64`. The package rewrites the app identifier and helper bundle IDs, bundles native libraries/N-API helpers, sets microphone and Apple Events usage descriptions, and includes `resources/entitlements.mac.plist`. Default signing is local ad-hoc signing, not distribution notarization. A Developer ID identity already installed in the user's keychain can be used explicitly:

```sh
SPEECHIFY_SIGN_IDENTITY='Developer ID Application: YOUR EXISTING IDENTITY' npm run package
```

For distribution, notarize the resulting DMG with your existing local keychain profile and staple it; this task does not provide an identity, submit a notarization request or publish a release. Do not disable Gatekeeper globally. For local ad-hoc builds, use Finder's ordinary Open workflow if macOS requests explicit trust.

Launch the built candidate directly, or copy **only this distinctly named app** to Applications:

```sh
open 'artifacts/speechify-electron-candidate-darwin-arm64/Speechify Electron Candidate.app'
```

In System Settings → Privacy & Security, grant this candidate Microphone, Accessibility and Input Monitoring access. System Events automation may also be requested for focus checks/paste. Restart after changing Input Monitoring. Initial default key is **Right Option**, minimum hold 250 ms. Models start uninstalled; choose a pinned native model. All data stays in `~/Library/Application Support/speechify-electron-candidate`. Python is unnecessary for the eleven native models. Local CUDA fine-tuning is unavailable on macOS; trained Whisper inference uses the optional CTranslate2 environment.

macOS runtime, focus, permissions, signing behavior and packaging commands are **not tested here, user-deferred**. In particular, see the current Mac focus-target limitation in R05; do not trust automatic paste with multiple windows of one app until that is resolved and tested.

## Windows: user-run build and launch

No Windows artifact was built on Linux. Use Windows 11 x64, Node 24, Python only for optional workers, PowerShell, and NSIS 3 if an installer is wanted. Run the common npm commands in PowerShell from `desktop/electron`. The package script produces an unpacked candidate and `artifacts/speechify-electron-candidate-windows-x64.zip`. Launch:

```powershell
& '.\artifacts\speechify-electron-candidate-win32-x64\speechify-electron-candidate.exe'
```

For the candidate-specific per-user installer, run `makensis windows-installer.nsi` from `desktop/electron/resources`. Its output is `speechify-electron-candidate-windows-x64-setup.exe`; it installs under `%LOCALAPPDATA%\Programs\speechify-electron-candidate`, with a distinctly named shortcut and uninstaller. Hash that installer after building (`Get-FileHash -Algorithm SHA256`). It does not replace Speechify. The installer was not compiled or run here.

Enable desktop microphone access in Windows Privacy settings. Default key is **Right Ctrl**. An unelevated candidate cannot inject into an elevated target due to UIPI; use matching ordinary privilege levels, not automatic elevation. All Windows runtime tests are **not tested, user-deferred**.

## Optional Python exception setup

Install Python 3.12 and uv 0.13.0+ as explicit local prerequisites, then:

```sh
node scripts/setup-python.mjs inference
node scripts/setup-python.mjs training
```

These create separate candidate-owned environments and synchronize the committed, universal, hashed lockfiles. Enter the printed executable paths in Settings → Optional Python workers. The training environment requires supported NVIDIA hardware/driver and a CUDA-enabled PyTorch installation from the lock's supported platform distribution. Windows/macOS wheel availability and actual CUDA usability have not been verified. Ordinary native dictation is unaffected if optional setup is unavailable.

On the Models page, **Install optional model** permits the Python worker to fetch to the candidate HF cache; **Use offline** forbids network via `HF_HUB_OFFLINE=1`. The three exception models retain their authoritative source IDs. Their model-install flow has documented parity gaps (no immutable HF revision receipts/byte progress). Candidate-trained `whisper-ft-*` directories containing `model.bin` are discovered automatically in the candidate `models/` directory. Do not move/delete original Speechify models.

## Reproduce fixture and failure-path checks without devices

The fixture CLI never launches the app or imports its main module. It verifies the model against the catalog and reads 16 kHz PCM16 WAV. Native libraries and model state remain loaded across repeated calls. The existing benchmark WAVs/models are only read. Fixture output contains actual ASR text; generic timing logs do not.

```sh
npm run build
node dist/fixture.js parakeet-tdt-0.6b-v3 \
  /home/bhatsumedh/bench-speechify-vs-handy/models/handy/parakeet-v3-Q8_0.gguf \
  /home/bhatsumedh/bench-speechify-vs-handy/audio/clips/short0.wav cpu 3
```

Run through the packaged executable:

```sh
ELECTRON_RUN_AS_NODE=1 \
  artifacts/speechify-electron-candidate-linux-x64/speechify-electron-candidate \
  artifacts/speechify-electron-candidate-linux-x64/resources/app/dist/fixture.js \
  parakeet-tdt-0.6b-v3 \
  /home/bhatsumedh/bench-speechify-vs-handy/models/handy/parakeet-v3-Q8_0.gguf \
  /home/bhatsumedh/bench-speechify-vs-handy/audio/clips/short0.wav cpu 3
```

For actual native CPU fallback, warm persistence, in-flight crash recovery and cancellation:

```sh
ELECTRON_RUN_AS_NODE=1 \
  artifacts/speechify-electron-candidate-linux-x64/speechify-electron-candidate \
  scripts/native-integration.cjs \
  artifacts/speechify-electron-candidate-linux-x64/resources/app \
  /home/bhatsumedh/bench-speechify-vs-handy/models/handy/parakeet-v3-Q8_0.gguf \
  /home/bhatsumedh/bench-speechify-vs-handy/audio/clips/short0.wav
```

Model SHA-256: `5859f77944efcd8eafa23a6350731960b2b55b2203df51f319665c807d802cc7`.
WAV SHA-256: `799f78ed4beb4de7ceae3a809262d4ce242394342ccd1d58cef7d49dbc2def46`.
Native Linux x64 wheel SHA-256: `9647bff7a7486810c2778fb9b86a6ac0302688620bfad8f31b560697423641aa`.

Evidence files under `docs/electron-evidence/` record final unit-test output, dependency audit, packaged fixture JSON and native integration JSON. Three fixture repeats are development evidence, **not** the contract's 20-dictation benchmark, not a cold filesystem-cache measurement, and not UI launch latency. The CLI reports worker model-ready time separately. A local Whisper file did not match the authoritative pin and was refused; Whisper inference is not claimed verified.

## Short manual acceptance checklist

Run one candidate at a time; quit the other before enabling hooks/microphone.

1. Start the packaged app. Confirm candidate-specific data paths, all 14 model rows, permissions guidance and no microphone use while idle. Select/install a pinned model, choose an input, then close Settings.
2. Dictate into a plain editor. Verify hold/release, Unicode/multiline text, focus preservation and clipboard restoration. Try short taps, Ctrl+C and another key during hold: no history/transcript/paste should appear. Switch target apps during inference: the result must be retained without a wrong-target paste. Treat any wrong-target paste as a release blocker.
3. Reopen/close Settings 20 times and perform at least 20 dictations. Record ready/window/mic/release/transcript/paste timestamps, process-tree idle CPU/RSS/PSS, and median/p95 separately for X11/Wayland/macOS/Windows. Do not enable always-open mode until idle-close behavior is verified.
4. Change the input, unplug it, rebind a shortcut, switch models, cancel a download, cancel an inference, then restart. Verify recovery and persistence. Check the indicator never takes focus.
5. Play/correct/revert/exclude/delete/export recordings. Confirm repeated corrections become rules and hints. If testing optional learning, use human/teacher-labeled audio and verify both gates before adoption; then roll back. Save-and-replace is explicitly unavailable in this candidate.
6. Enable and disable candidate startup only if desired, confirm the original Speechify entry remains unchanged, then quit. Confirm worker/helper processes are gone and the microphone indicator clears. Report unsupported Wayland global hooks/paste separately rather than attempting privilege escalation.

## Independent lead review

The lead reran the packaged Parakeet CPU fixture (see `electron-evidence/lead-packaged-fixture.json`) and the unit suite. Review found and fixed filtering after the history result cap, which hid old matching entries, and acceptance of non-finite teacher confidence. The expanded suite passes 23 tests. Fixture timings are not desktop latency measurements or controlled candidate rankings. Linux desktop integration is still unverified: this host currently has only a GDM greeter session, not an accessible logged-in user desktop.

The lead attempted the corrected packaged GUI on an isolated Xvfb display. Chromium aborted before any window opened because the portable sandbox helper is not root-owned/setuid and the host restricts unprivileged user namespaces. No GUI, tray, microphone, keyboard-hook, focus, or paste pass is claimed from that attempt. Existing services and the GDM greeter were not changed. Checksums were regenerated after the lead fixes and both packages passed `sha256sum -c SHA256SUMS`.
