# Speechify

Hold a key, talk, let go, and the text is pasted wherever your cursor is.
Speechify runs speech recognition locally on Windows, Linux and macOS, lets you
pick from 14 speech models, and learns from your corrections.

It is a [Tauri](https://tauri.app) app: a Rust service that owns recording,
inference, history and learning, and a TypeScript window for settings,
history, models and learning.

## Install

Download the file for your system from the
[latest release](https://github.com/Sumedhbhat/speechify/releases/latest):

| System | File |
| --- | --- |
| Windows 10 and 11, x64 | `Speechify-<version>-windows-x64-installer.exe` |
| macOS 12+, Apple silicon | `Speechify-<version>-mac-arm64.dmg` |
| Ubuntu 24.04+, and distributions with its `t64` GTK and ALSA packages | `Speechify-<version>-linux-amd64.deb` (`sudo apt install ./Speechify-*.deb`) |
| Other Linux, x86_64 | `Speechify-<version>-linux-x86_64.tar.gz` (portable; needs system GTK, WebKitGTK and ALSA) |

Nightly builds from `main` are prereleases on the
[releases page](https://github.com/Sumedhbhat/speechify/releases). To update,
install the new version over the old one; your data is kept. See
[docs/releases.md](docs/releases.md).

Settings shows the installed version, and so does `speechify --version`.

## Using it

Hold the dictation key (Right Ctrl on Windows and Linux, Right Option on macOS)
for at least 250 ms, speak, and let go. You can change the key in Settings.
**Record in app** works without any global-key permission; copy the text
afterwards. Closing the window keeps Speechify running in the tray; **Quit
Speechify** in the tray stops it.

The first time, choose and download a model in **Models**. Ordinary models run
through [transcribe.cpp](https://github.com/handy-computer/transcribe.cpp)
without Python, on the GPU when one is available and on the CPU otherwise.
Distil-Whisper, Kyutai and trained models need an optional Python environment,
which **Models** installs on request (Python 3 and `uv` must be on `PATH`).

**On Windows**, allow microphone access for desktop apps. Windows doesn't let a
normal app paste into an app running as administrator.

**On macOS**, the app is ad-hoc signed until it has an Apple Developer ID, so the
first open needs System Settings > Privacy & Security > **Open Anyway**. Grant
Microphone, Accessibility and Input Monitoring, then restart Speechify.

**On Linux with X11**, pasting needs `xclip` and `xdotool` (the `.deb` installs
them). **On Wayland**, apps can't see each other's keys or focus, so the global
key needs read access to `/dev/input/by-id/*-event-kbd`, which you grant
yourself, and Speechify saves the text for copying instead of pasting it
(copying needs `wl-clipboard`). Speechify never changes permissions for you.

## Your data

Recordings, history, settings, models and learning data live in one folder:

| System | Folder |
| --- | --- |
| Linux | `$XDG_DATA_HOME/org.sumedhbhat.speechify`, or `~/.local/share/org.sumedhbhat.speechify` when `XDG_DATA_HOME` isn't set |
| macOS | `~/Library/Application Support/org.sumedhbhat.speechify` |
| Windows | `%APPDATA%\org.sumedhbhat.speechify` |

Builds from before the rename used a folder named `speechify-tauri-candidate`;
Speechify moves it to the new name the first time it starts. Data from the
older Python version of Speechify (the `speechify` folder) can be copied in
with **Settings > Copy existing Speechify data**.

## Developing

- [docs/building.md](docs/building.md): prerequisites and builds for each system.
- [docs/architecture.md](docs/architecture.md): where the code lives and how a dictation flows through it.
- [docs/testing.md](docs/testing.md): the behavioral tests and the full-coverage gate.
- [docs/releases.md](docs/releases.md): nightly and stable releases.

```sh
npm ci
python3 scripts/prepare-native.py   # `python` on Windows
npm run tauri dev       # run the app
npm run check           # formatting, tests, build
```
