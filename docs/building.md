# Building

## What you need

| Tool | Version | Used for |
| --- | --- | --- |
| Rust | 1.99.0 | Pinned in `rust-toolchain.toml`; rustup installs it, with rustfmt, clippy and llvm-tools |
| Node | 24 (CI uses 24.21.0) | The interface and the Tauri CLI |
| Python 3 | any recent 3 | Only `scripts/fetch-native.py` and the helper scripts. Standard library only |

Linux also needs development packages for WebKitGTK, GTK and ALSA. These are the ones CI installs (Ubuntu 24.04):

```
sudo apt-get update
sudo apt-get install -y libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev \
  libasound2-dev libx11-dev libxtst-dev libxi-dev
```

Windows and macOS need the usual toolchains (MSVC build tools and WebView2 on Windows, Xcode command line tools on macOS) and nothing else.

## Build and run from source

```
python3 scripts/fetch-native.py      # speech runtime into src-tauri/resources/native
npm ci
npm run tauri -- dev                 # Vite dev server plus the app
```

On Windows use `python` for the first command.

`fetch-native.py` downloads the pinned transcribe.cpp 0.4.1 wheel for your platform from PyPI, checks its SHA-256, and unpacks only the native libraries and licences. It installs nothing into Python. Running it again is a no-op when `provenance.json` already matches. Supported platforms: Linux x86_64 and aarch64, macOS arm64 and x86_64, Windows x64. The runtime includes the CPU, Vulkan and Metal backends. The folder is git-ignored.

To run against a runtime somewhere else, set `SPEECHIFY_NATIVE_DIR`.

The app downloads speech models itself, from the Models page. Nothing in the build needs a model, except the tests (see [testing.md](testing.md)).

## Without root on Linux

If you cannot `apt install`, unpack the packages into a folder and point the compiler at it:

```
python3 scripts/bootstrap-linux.py            # into .build/sysroot
source scripts/local-env.sh                   # sets PKG_CONFIG_PATH, LIBRARY_PATH, RUSTFLAGS and more
cargo build -p speechify
```

`bootstrap-linux.py` runs `apt-get -s install` to find what is missing, downloads the `.deb` files with `apt-get download`, and extracts them with `dpkg-deb -x`. Nothing is installed system-wide. `local-env.sh` also limits `CARGO_BUILD_JOBS` to 3 unless you set it.

## Check the build

```
npm run build                         # tsc --noEmit, then vite build into dist/
cargo build -p speechify --locked     # what CI runs
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
npm run format:check
```

`npm run format` rewrites the interface files with Prettier. `cargo fmt --all` formats Rust.

## Make an installer

The release workflow is the reference. It sets `SPEECHIFY_VERSION`, which `speechify --version` and the Settings page read at compile time.

```
python3 scripts/fetch-native.py
npm ci
export SPEECHIFY_VERSION=0.1.0
npm run tauri -- build --ci --config "{\"version\":\"$SPEECHIFY_VERSION\"}" --bundles deb -- --locked    # Linux
npm run tauri -- build --ci --config "{\"version\":\"$SPEECHIFY_VERSION\"}" --bundles app -- --locked    # macOS
npm run tauri -- build --ci --config "{\"version\":\"$SPEECHIFY_VERSION\"}" --bundles nsis -- --locked   # Windows
```

Output lands in `target/release/bundle/`. The Linux package depends on `libasound2t64`. The macOS release job signs the `.app` ad hoc with `src-tauri/Entitlements.plist` and wraps it in a `.dmg`. Nothing is signed for distribution yet, so Windows and macOS will warn about an unidentified publisher.

## Where things are

- Cargo workspace root: `Cargo.toml` (members `core`, `os`, `src-tauri`). The version lives only here.
- Tauri config: `src-tauri/tauri.conf.json`. Windows are `main` (`index.html`) and `bar` (`bar.html`).
- The bundled runtime is listed under `bundle.resources` as `resources/native/**/*`.
- Dependencies are pinned to exact versions in the manifests and locked in `Cargo.lock` and `package-lock.json`. Use `--locked` and `npm ci`.
