# Releases

Speechify ships on two channels. Each comes from its own kind of branch, so a
stable line can get fixes without taking everything that has landed on `main`
since.

| Channel | Branch | Version | GitHub release |
| --- | --- | --- | --- |
| Nightly | `main` | `<Cargo.toml version>-nightly.<CI run>` | Prerelease such as `v0.2.0-nightly.57`, one per push to `main` (except `[no-release]` merges) |
| Stable | `release/X.Y` | The `Cargo.toml` version, always `X.Y.Z` | `vX.Y.Z`, published when that version first lands on the branch |

The version is written in one place: `version` in
[`src-tauri/Cargo.toml`](../src-tauri/Cargo.toml). `tauri.conf.json` and
`package.json` don't carry one. CI adds the `-nightly.<run>` suffix at build
time and never edits the manifest.

## What a release contains

| Asset | What it is |
| --- | --- |
| `Speechify-<version>-windows-x64-installer.exe` | Windows installer (NSIS) |
| `Speechify-<version>-mac-arm64.dmg` | macOS app for Apple silicon, ad-hoc signed |
| `Speechify-<version>-linux-amd64.deb` | Ubuntu 24.04 or newer, and Debian-based distributions with the same libraries |
| `Speechify-<version>-linux-x86_64.tar.gz` | Portable Linux build: the executable, native inference libraries, optional Python resources, `VERSION` and `SHA256SUMS`. Needs the system GTK 3, WebKitGTK 4.1 and ALSA libraries. |
| `SHA256SUMS` | Checksums of all of the above |

Speech models aren't included. Each one is downloaded the first time you pick
it in **Models**.

The macOS app isn't signed with an Apple Developer ID, so Gatekeeper asks
before the first start, and macOS may ask for its permissions again after an
update. Signing and notarizing with a Developer ID would fix both.

## Updating

Install the new version over the old one. The installers replace the app and
never touch the data folder (see "Your data" in the README). Speechify
doesn't update itself yet.

Releases up to `v0.1.0-nightly.73` were the older Python version of Speechify. Its tray updater
looks for that app's installer names, so it won't offer these releases; install
this version by hand once. The two apps keep their data in different folders,
and **Settings > Copy existing Speechify data** copies the old history across.

## Which version am I running?

- **Settings > Data and diagnostics**
- `speechify --version`
- the `VERSION` file in the portable archive
- the release title on GitHub

Every release is permanent. Tags and assets are never moved, deleted, or
overwritten; a fix is always a new patch version.

## Cut a stable release

`main` always carries the next minor version, so nightlies sort after the
newest stable release.

1. Check that the version in `src-tauri/Cargo.toml` on `main` is `X.Y.0`.
2. Create and push the branch:

   ```bash
   git switch -c release/X.Y main && git push -u origin release/X.Y
   ```

   CI publishes `vX.Y.0`.
3. Open a PR on `main` that bumps `src-tauri/Cargo.toml` to `X.(Y+1).0`
   (and run `cargo update -p speechify` so `Cargo.lock` matches).

Release branches are always `release/<major>.<minor>`, never `release/0.1.0`:
one branch carries every patch release of its line. CI rejects a branch whose
version doesn't start with `X.Y.`.

## Fix a released version

1. Branch from the release line: `git switch -c fix/<topic> origin/release/X.Y`.
2. Make the fix and bump `src-tauri/Cargo.toml` to `X.Y.(Z+1)` in the same PR.
3. Open the PR against `release/X.Y`, not `main`. Merging it publishes
   `vX.Y.(Z+1)`. A push that leaves the version unchanged builds but doesn't
   publish.
4. If `main` has the same bug, open a separate PR that cherry-picks the fix
   onto `main`. Never merge a release branch back into `main`.

## How CI does it

Two workflows, one job each for every step:

- [`ci.yml`](../.github/workflows/ci.yml) runs the behavioral tests on Linux,
  macOS and Windows and then requires full coverage. Pull requests run it
  directly.
- [`release.yml`](../.github/workflows/release.yml) runs on pull requests and
  on every push to `main` and `release/**`:
  1. **Version**: reads `src-tauri/Cargo.toml` and picks the channel from the
     branch.
  2. **Tests** (pushes only): runs `ci.yml`.
  3. **Package**: builds the installer on each system with the version
     stamped in, and checks that `speechify --version` prints it.
  4. **Publish** (pushes only, never pull requests): after tests and packages
     pass, creates the GitHub release with the installers and `SHA256SUMS`.
     Nightlies are prereleases. If a stable tag already exists, it skips
     publishing and says why in the log.

To merge without publishing, put `[no-release]` in the merge or squash commit
message. Packages are still built and kept as workflow artifacts.

## Dependencies

- Security fixes merge as soon as CI passes. A security fix that bumps a major
  version gets a PR right away, labelled `security-major` for same-day review.
- Every other new version waits 14 days after release
  ([`.github/dependabot.yml`](../.github/dependabot.yml) covers npm, Cargo and
  GitHub Actions). The optional Python environments use hash-locked files
  compiled with `--exclude-newer`.
- Overrides of transitive dependencies are a last resort. Each one is logged
  in [`.github/dependency-overrides.yml`](../.github/dependency-overrides.yml).

The `DEPENDABOT_ALERTS_TOKEN` Dependabot secret (a fine-grained token with
read-only Dependabot alerts access to this repository) lets the security
workflow tell security PRs apart so they can auto-merge. Without it, nothing
auto-merges and the workflow says so in a warning:

```bash
gh secret set DEPENDABOT_ALERTS_TOKEN --app dependabot --repo Sumedhbhat/speechify
```
