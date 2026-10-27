# Speechify Tauri Candidate

Rust service and TypeScript settings UI. App ID: `org.sumedhbhat.speechify.tauri`.
This is a comparison candidate with documented parity gaps, not a replacement for the installed Python application.

The Rust process owns recording, keyboard polling, the bounded dictation queue, native worker supervision, SQLite, downloads and learning. Closing Settings hides the window; Quit stops the service. Ordinary dictation uses transcribe.cpp 0.3.1 without Python. Native models are persistent in a separate Rust child process, with CPU retry after GPU failure. All 14 original catalog entries remain available; Distil-Whisper, Kyutai and trained CTranslate2 models use an explicit optional Python worker.

## Build on Linux

Prerequisites: Rust stable (tested with 1.99.0), Node 22.12+ or 24+, npm, Python 3 (build-time resource preparation only), a C/C++ compiler, pkg-config, GTK 3, WebKitGTK 4.1, ALSA and Ayatana appindicator development packages. Runtime global dictation on X11 additionally requires xclip, xdotool, libXfixes and coreutils timeout. Wayland global recording is available only with explicitly granted read access to `/dev/input/by-id/*-event-kbd`. Record in app/Stop and transcribe works without global input grants. Automatic Wayland paste is refused because safe target discovery is unavailable; use Copy text or Copy in History (requires wl-clipboard). No permissions are changed automatically.

From `desktop/tauri`:

```sh
npm ci
python3 scripts/prepare-native.py
npm run build
cd src-tauri
cargo test --locked --no-default-features
cargo check --locked
cd ..
npm run tauri -- build --ci --bundles deb -- --locked
python3 scripts/package-portable.py
```

For this machine's locally extracted prerequisites, first run `python3 scripts/bootstrap-linux.py` and `source scripts/local-env.sh`. These only extract packages into `.build/`; they do not install packages or change services. The source checkout's build evidence documents the exact environment.

The Linux portable tar contains the release executable, native inference libraries, optional-worker/training resources and checksums. It requires the system GTK/WebKit/ALSA libraries; it is not a universal static Linux binary. Build on your oldest supported Linux distribution for broader glibc compatibility. A binary built on Ubuntu 26.04 is not claimed to run on older glibc distributions.

## Build and launch on your Mac

Use a native Apple Silicon or Intel Mac, macOS 12+, Xcode Command Line Tools (`xcode-select --install`), Rust stable for that architecture, Node 24 LTS/npm, and Python 3. Build each architecture separately; the native runtime resources are architecture-specific. No macOS binary was produced on the Linux build machine.

```sh
cd desktop/tauri
npm ci
python3 scripts/prepare-native.py
npm run build
cd src-tauri
cargo test --locked --no-default-features
cargo check --locked
cd ..
npm run tauri -- build --ci --bundles app,dmg -- --locked
open 'src-tauri/target/release/bundle/macos/Speechify Tauri Candidate.app'
```

The resulting `.app` is in `src-tauri/target/release/bundle/macos/`; the `.dmg` is in `src-tauri/target/release/bundle/dmg/`. Keep the candidate name and identifier. Do not replace existing Speechify. Microphone usage text and audio-input entitlement are included. Give this candidate Microphone, Accessibility and Input Monitoring permission in System Settings → Privacy & Security, then restart it. Default hold key is Right Option. The polling backend refuses global dictation until its required permissions are granted.

For distribution, set `APPLE_SIGNING_IDENTITY` to your Developer ID identity before building and use Tauri's standard notarization credentials. Sign the nested native dylibs and the app with the same identity; verify `codesign --verify --deep --strict` and `spctl --assess --type execute` before sharing. Unsigned/local builds are for your own testing; changing build/signing identity can require renewing permission grants. Signing and notarization were not performed on Linux.

## Windows build

Use Windows 10/11 x64, Rust stable with MSVC, Visual Studio C++ Build Tools + Windows SDK, Node 24/npm, Python 3 and WebView2 Runtime. Run the same resource preparation and frontend commands, then:

```powershell
npm run tauri -- build --ci --bundles nsis -- --locked
```

The installer goes under `src-tauri/target/release/bundle/nsis/`. Build and runtime validation are user-deferred. Enable Windows microphone access for desktop apps. UIPI prevents a normal candidate from inserting into an elevated target; do not elevate automatically. Native Windows input and clipboard adapters exist but have not been exercised here.

## Fixture-only inference

These commands do not initialize a tray, microphone or global input:

```sh
./speechify-tauri-candidate diagnostics
./speechify-tauri-candidate install-model moonshine-base
./speechify-tauri-candidate fixture moonshine-base /absolute/path/to/jfk.wav 20 cpu
```

`SPEECHIFY_CANDIDATE_PARENT=/absolute/test-parent` places all candidate data below that parent in `speechify-tauri-candidate`. The normal data root is `~/.local/share/speechify-tauri-candidate` on Linux, `~/Library/Application Support/speechify-tauri-candidate` on macOS and `%APPDATA%\speechify-tauri-candidate` on Windows. `SPEECHIFY_HOME` is deliberately ignored. `SPEECHIFY_NATIVE_DIR` may identify an explicitly prepared native resource directory for fixture tests.

Each fixture result prints JSON with actual text, device and inference milliseconds. `logs/timings.jsonl` records model-ready/load timing and inference timing without transcript text. Cold process launch includes checksum validation and model loading; fixture iteration time is inference, not cold-cache or paste latency.

## Optional Python exceptions

Install Python 3 and `uv` explicitly on PATH. In Models choose **Install optional model**; in Learning choose **Install training environment**. Stages and byte progress appear in the app. Cancel stops the owned process tree; Retry repairs incomplete environments and restarts unverified model files. Completion is atomic for model directories; dependency installation uses a ready marker. These operations only use candidate-owned data. The checked-in hash/date-constrained lockfiles are unchanged. The existing CLI remains available:

```sh
python3 scripts/setup-exception.py whisper --model distil-whisper-large-v3
python3 scripts/setup-exception.py transformers --model kyutai-stt-1b
python3 scripts/setup-exception.py transformers --model kyutai-stt-2.6b
python3 scripts/setup-exception.py training
```

Python 3.12 environments are separate because training uses transformers 4.x and Kyutai uses 5.x. Training requires NVIDIA CUDA and roughly 10 GB VRAM, as in the original trainer. The trainer runs only inside candidate data and cannot adopt a model itself. Rust requires both personal held-out WER and LibriSpeech gates before switching. Trained CTranslate2 inference additionally needs the `whisper` environment.

These optional multi-gigabyte environments/models were not installed or executed during fixture development. Their lockfiles, pinned model revisions and checksums, resource packaging, process protocol and lifecycle controls are implemented. No ordinary UI or native-model lifecycle depends on them. Replacing these exceptions requires native Kyutai support, a pinned native Distil conversion, and a native training/export pipeline.

## User device checklist

1. Confirm the candidate name/data directory; leave the existing app's data untouched. Install/select one model and disconnect the network after installation.
2. Grant the platform permissions. Select the microphone; hold Right Option on Mac or Right Ctrl on Windows/X11 for at least 250 ms. Verify Unicode/multiline output in a plain text editor.
3. Tap the key briefly, hold it with Ctrl+C/other keys, and rebind it. No cancelled clip should reach history or paste. Unplug the selected microphone and inspect recovery/error guidance.
4. Close/reopen Settings 20 times and dictate 20 clips. Verify recording and inference persist independently of the settings window. Check the tray's Settings/History/Models/Quit actions and second-instance behavior.
5. Change focus while transcription runs: the transcript should be saved without pasting into the new target. Copy both different and identical text during clipboard restoration; verify newer copies survive. Test HTML/RTF and unsupported-format refusal. Check indicator visibility without focus activation.
6. Correct, revert, exclude/include, play, search and export history; repeat a word correction twice and inspect its rule. With audio retention off, correct the latest clip before another dictation/restart and verify its WAV is then saved. Save-and-replace supports only a standard Windows Edit control with unchanged text/caret; other targets save the correction and report refusal.
7. Cancel a download, retry, select a different installed model, restart, and quit during inference. Confirm no worker remains. Check CPU fallback with an unavailable GPU choice.
8. If testing learning, opt in to a teacher/CPU as needed, verify mains/idle scheduling and dictation preemption; train only after enough labels. Review both gate reports and test rollback.

See repository `docs/candidate-parity.md` and `docs/candidate-testing.md` for exact verified evidence and remaining gaps. No live-device, tray, microphone, permission or focus verification is implied by compilation or fixture inference.


## Clipboard and correction limits

X11 restoration uses unique selection ownership and retains supported MIME data, up to 128 KiB per target. Large/unknown formats refuse automatic paste; there is no INCR or clipboard-manager handoff yet. Windows snapshots memory-backed formats under the clipboard lock; handle formats refuse automatic paste. Mac snapshots NSPasteboard item types and guards by change count, but AppKit has no atomic compare-and-restore operation. Native runtime checks are still required.

Windows replacement is limited to standard Edit controls, an unchanged UTF-16 document/caret and a two-minute receipt. Replacement uses bounded synchronous messages and is undoable. It does not move focus back to the original app. Concurrent programmatic mutation between separate Win32 messages remains a limitation; other control types and platforms save only and explain the refusal.

The lead owns native macOS packaging via GitHub Actions after this handoff. No CI workflow, branch push, installed-app replacement or release is part of this implementation commit.
