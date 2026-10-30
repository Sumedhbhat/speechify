# Building Speechify

Every build needs Rust stable, Node 24 (or 22.12+) with npm, and Python 3.
Python is used at build time to fetch the pinned native inference libraries;
ordinary dictation doesn't use it. Run everything from the repository root.

```sh
npm ci
python3 scripts/prepare-native.py   # transcribe.cpp 0.3.1 libraries, checksum-verified
npm run tauri dev                   # run the app from source
```

`npm run check` runs formatting, the frontend tests with full coverage, the
frontend build and the Rust suites. [testing.md](testing.md) covers the native
coverage run.

## Linux

Install a C/C++ compiler, pkg-config, and the GTK 3, WebKitGTK 4.1, ALSA and
Ayatana appindicator development packages. On Ubuntu:

```sh
sudo apt-get install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libasound2-dev libx11-dev libxtst-dev
```

Then build the Debian package and the portable archive:

```sh
npm run tauri -- build --ci --bundles deb -- --locked
python3 scripts/package-portable.py
```

The `.deb` is in `src-tauri/target/release/bundle/deb/` and the archive in
`artifacts/`. Released Linux builds come from Ubuntu 24.04 and need its
libraries (the `t64` GTK and ALSA packages); on older distributions, build from
source. The portable archive uses the system GTK, WebKitGTK and ALSA
libraries; it is not a static binary.

Global dictation on X11 needs `xclip`, `xdotool`, libXfixes and coreutils
`timeout` at run time. On Wayland, see "Using it" in the README.

On a machine where you can't install packages, `python3 scripts/bootstrap-linux.py`
extracts the build prerequisites into `.build/`, and `source scripts/local-env.sh`
points the compilers at them. Neither installs anything or changes services.

## macOS

Use a native Apple silicon or Intel Mac on macOS 12+, with the Xcode Command
Line Tools (`xcode-select --install`) and Rust for that architecture. The
native libraries are architecture-specific, so build each architecture on its
own machine.

```sh
npm run tauri -- build --ci --bundles app,dmg -- --locked
open src-tauri/target/release/bundle/macos/Speechify.app
```

Grant Microphone, Accessibility and Input Monitoring in System Settings >
Privacy & Security, then restart the app. Changing the signing identity can
require granting them again.

For distribution, set `APPLE_SIGNING_IDENTITY` to a Developer ID identity
before building and use Tauri's notarization credentials. Sign the nested
native dylibs and the app with the same identity, and check with
`codesign --verify --deep --strict` and `spctl --assess --type execute`.

## Windows

Use Windows 10/11 x64 with Rust (MSVC), the Visual Studio C++ Build Tools and
Windows SDK, and the WebView2 Runtime. Run the same preparation commands
(`python` instead of `python3` if that's how Python is installed), then:

```powershell
npm run tauri -- build --ci --bundles nsis -- --locked
```

The installer is in `src-tauri/target/release/bundle/nsis/`.

## Command-line tools

The built executable has commands that don't open a window, tray, microphone
or global input:

```sh
speechify --version
speechify diagnostics
speechify install-model moonshine-base
speechify fixture moonshine-base /absolute/path/to/jfk.wav 20 cpu
```

`fixture` prints one JSON line per run with the text, device and inference
time, without adding to history. `logs/timings.jsonl` in the data folder records
model loading and inference timings without transcript text.

`SPEECHIFY_DATA_PARENT=/absolute/test-parent` puts the data folder under that
parent instead of the system location; the tests use it to stay isolated.
`SPEECHIFY_NATIVE_DIR` points at an explicitly prepared native library folder.

## Optional Python models and training

Distil-Whisper, Kyutai and fine-tuned Whisper models, and training itself, run
in separate Python 3.12 environments. Install Python 3 and `uv` on `PATH`, then
use **Models > Install optional model** or **Learning > Install training
environment**. Progress shows in the app; Cancel stops the whole process tree,
and Retry repairs an incomplete environment. The same installs are available
from a source checkout:

```sh
python3 scripts/setup-exception.py whisper --model distil-whisper-large-v3
python3 scripts/setup-exception.py transformers --model kyutai-stt-1b
python3 scripts/setup-exception.py transformers --model kyutai-stt-2.6b
python3 scripts/setup-exception.py training
```

Training and Kyutai use separate environments because they need transformers
4.x and 5.x. Training needs an NVIDIA GPU with about 10 GB of VRAM and isn't
available on macOS. A trained model is adopted only after it passes both the
personal held-out and LibriSpeech gates.

The lockfiles, pinned model revisions and checksums are in
`src-tauri/resources/`. `scripts/pin-exception-models.py` refreshes the model
revisions. Regenerate the training lock with:

```sh
uv pip compile --python-version 3.12 --universal --generate-hashes \
  --exclude-newer <date> --torch-backend cu126 --emit-index-url \
  src-tauri/resources/requirements/training.in \
  -o src-tauri/resources/requirements/training.lock
```

## Known limits

- **Clipboard.** X11 restores supported clipboard formats up to 128 KiB each;
  larger or unknown formats make Speechify refuse to paste automatically.
  Windows restores memory-backed formats only. macOS checks the pasteboard
  change count, but AppKit has no atomic compare-and-restore.
- **Correct and replace.** Replacing pasted text with a correction works only in
  a standard Windows Edit control whose text and caret haven't changed, within
  two minutes. Elsewhere the correction is saved and the app says why it
  didn't replace the text.
- **Wayland.** Speechify doesn't paste automatically, because it can't safely
  find the target window. Use Copy text.
