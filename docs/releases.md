# Releases

Speechify ships on two channels. Each comes from its own kind of branch, so a
stable line can get fixes without taking everything that has landed on `main`
since.

| Channel | Branch | Version | GitHub release |
| --- | --- | --- | --- |
| Nightly | `main` | `<pyproject version>-nightly.<CI run>` | Prerelease such as `v0.2.0-nightly.57`, one per push to `main` |
| Stable | `release/X.Y` | The pyproject version, always `X.Y.Z` | `vX.Y.Z`, published when that version first lands on the branch |

All releases are on the [releases page](https://github.com/Sumedhbhat/speechify/releases).

## Installing a channel

Stable releases are under
[latest release](https://github.com/Sumedhbhat/speechify/releases/latest);
nightlies and older versions are further down the
[releases page](https://github.com/Sumedhbhat/speechify/releases). Every
release has the same four installers (see below); download the one for your
system and run it.

Updating is the same: install the new version over the old one, or switch
channels by installing a version from the other channel. The installer
replaces the app (in `%LOCALAPPDATA%\Programs\Speechify` on Windows,
`/Applications/Speechify.app` on macOS, `/opt/speechify` for the `.deb`) and
never touches the data folder (`%APPDATA%\speechify`,
`~/Library/Application Support/speechify` or `~/.local/share/speechify`),
where your recordings, corrections, settings, model environments and
fine-tuned models live. Going back to an older version works the same way,
except with the `.deb`, where apt needs `--allow-downgrades`.

### Coming from install.sh or install.ps1

Releases before the installers used scripts that put Speechify in
`~/.local/share/speechify-app` (Linux, macOS) or
`%LOCALAPPDATA%\Programs\Speechify` (Windows). Your data is in the same place
either way, so nothing needs moving.

- Windows: run the new installer. It cleans out the old files in the same folder.
- Linux: the new app menu entry replaces the old one. Delete
  `~/.local/share/speechify-app` and `~/.local/bin/speechify`.
- macOS: delete `~/Applications/Speechify.app`, `~/.local/share/speechify-app`
  and `~/.local/bin/speechify`, plus
  `~/Library/LaunchAgents/io.github.sumedhbhat.speechify.plist` if you used
  `--autostart`.

## Which version am I running?

- `speechify --version`
- the first line Speechify prints when it starts
- the `VERSION` file in the install folder
- the release title on GitHub

Every release is permanent. Tags and assets are never moved, deleted, or
overwritten; a fix is always a new patch version.

## What a release contains

| Asset | What it is |
| --- | --- |
| `Speechify-<version>-windows-x64-setup.exe` | Windows installer (Inno Setup). Per-user, no admin needed. |
| `Speechify-<version>-macos-arm64.dmg` | macOS app for Apple silicon |
| `speechify_<version>_amd64.deb` | Ubuntu, Debian and derivatives. Nightlies are `<version>~nightly.<run>` so apt sorts them before the stable release. |
| `Speechify-<version>-x86_64.AppImage` | Any Linux with glibc 2.35 or newer |
| `SHA256SUMS` | Checksums of all four |

Each one holds the app frozen by PyInstaller with its own Python, plus
[uv](https://docs.astral.sh/uv/), which builds each speech model's
environment the first time you pick it. The Linux builds also carry PortAudio,
built without JACK so it only needs ALSA. `packaging/build.sh <version>` builds
the installer for the platform it runs on.

Speech models aren't in the installers. Each one is downloaded the first time
you pick it (see "Models" in the README).

The macOS app is ad-hoc signed, not with an Apple Developer ID, so Gatekeeper
asks before the first start, and macOS forgets its Microphone, Input
Monitoring and Accessibility permissions on every update. Signing and
notarizing with a Developer ID would fix both.

## Cut a stable release

`main` always carries the next minor version, so nightlies sort after the
newest stable release.

1. Check that the version in `pyproject.toml` on `main` is `X.Y.0`.
2. Create and push the branch:

   ```bash
   git switch -c release/X.Y main && git push -u origin release/X.Y
   ```

   CI publishes `vX.Y.0`.
3. Open a PR on `main` that bumps `pyproject.toml` to `X.(Y+1).0`.

Release branches are always `release/<major>.<minor>`, never
`release/0.1.0`: one branch carries every patch release of its line. CI
rejects a branch whose `pyproject.toml` version doesn't start with `X.Y.`.

## Fix a released version

1. Branch from the release line: `git switch -c fix/<topic> origin/release/X.Y`.
2. Make the fix and bump `pyproject.toml` to `X.Y.(Z+1)` in the same PR.
3. Open the PR against `release/X.Y`, not `main`. Merging it publishes
   `vX.Y.(Z+1)`. A push that leaves the version unchanged builds but doesn't
   publish.
4. If `main` has the same bug, open a separate PR that cherry-picks the fix
   onto `main`. Never merge a release branch back into `main`.

Several release lines can be supported at once; `release/0.1` and
`release/0.2` each take fixes independently.

## How CI does it

[`.github/workflows/release.yml`](../.github/workflows/release.yml) runs on
every push to `main` and `release/**`, and on pull requests:

1. **Version**: reads `pyproject.toml` and picks the channel from the branch.
   On `main` it appends `-nightly.<run>`; it never edits the manifest.
2. **Package**: builds the installers with `packaging/build.sh`, on Windows,
   macOS and Ubuntu 22.04 (the oldest glibc the Linux builds support).
3. **Check package**: installs each one on a fresh runner (Ubuntu 24.04 for
   Linux), checks `speechify --version` prints the stamped version, and runs
   `speechify selftest --engine-only` through the installed app.
4. **Publish** (pushes only, never PRs): creates the GitHub release with the
   installers and `SHA256SUMS`. Nightlies are marked as prereleases. If a
   stable tag already exists, it skips publishing and says why in the log.

## Dependencies

Dependency updates follow the repo policy:

- Security fixes merge as soon as CI passes. A security fix that bumps a major
  version gets a PR right away, labelled `security-major` for same-day review.
- Every other new version waits 14 days after release
  ([`.github/dependabot.yml`](../.github/dependabot.yml)). The same 14-day rule
  applies to the packages Speechify installs for each model, through uv's
  `--exclude-newer`.
- Overrides of transitive dependencies are a last resort. Each one is logged
  in [`.github/dependency-overrides.yml`](../.github/dependency-overrides.yml),
  and a daily job opens a PR to remove it once upstream has caught up.

## Repository secrets

| Secret | Kind | Token permissions | Used for |
| --- | --- | --- | --- |
| `DEPENDABOT_ALERTS_TOKEN` | Dependabot secret | Dependabot alerts: read-only | Telling security PRs apart so they can auto-merge. Without it, nothing auto-merges and the workflow says so in a warning. |
| `DEPS_BOT_TOKEN` | Actions secret | Contents, Pull requests, Issues: read/write | Opening PRs that remove overrides upstream no longer needs. |
| `HF_TOKEN` | Actions secret | Hugging Face read token from an account that accepted Cohere Transcribe's terms | Testing the gated Cohere model in the Models workflow. Without it, that one model is skipped. |

Use fine-grained tokens limited to this repository:

```bash
gh secret set DEPENDABOT_ALERTS_TOKEN --app dependabot --repo Sumedhbhat/speechify
gh secret set DEPS_BOT_TOKEN --repo Sumedhbhat/speechify
gh secret set HF_TOKEN --repo Sumedhbhat/speechify
```

## Verification

Nothing is published unless `speechify selftest` passes on a Windows runner
(the `selftest` job in `release.yml`), and `speechify selftest --engine-only`
passes in the installed app on Windows, Linux and macOS (`check-package`):
the CPU packages and default model a machine without an NVIDIA GPU gets. It runs the real app with a recorded
clip standing in for the microphone and checks, with a screenshot per step:

- holding Right Ctrl transcribes the clip and pastes it into Notepad, with
  capital letters and punctuation (`metrics.formatting` against a punctuated
  reference; word error rate can't see them)
- the overlay never takes focus from the app you're dictating into
- Ctrl+Alt+F opens the fix window, a pasted fix replaces the text, and
  Ctrl+Enter learns it and swaps it in Notepad
- the next dictation applies the learned fix
- Ctrl+Alt+H lists both recordings, and Ctrl+Alt+M lists every model

The report and screenshots are attached to each run as the `selftest-report`
artifact. The [Models workflow](../.github/workflows/models.yml) separately
installs and runs every model on Windows, and one per backend on Linux and
macOS, weekly and whenever engine code changes, and fails a model whose
transcripts lose their capitals or punctuation. CI also types through a real
uinput virtual keyboard and reads it back from `/dev/input`, the path
Speechify takes on Wayland. What CI can't cover is a desktop session on
Linux or macOS: grabbing the talk key and pasting into another app there have
only been tested by hand. Runners have no GPU, so GPU speed is the one thing CI can't measure;
run `speechify selftest` on your own machine for that.

## Merge without publishing

For a merge that should run packaging and checks without publishing a release,
include `[no-release]` in the final merge or squash commit message. The Release
workflow still builds and tests the installers; its publish job is skipped.
Tauri desktop packages are separate CI artifacts and are never published by
that Python release workflow.
