# Releases

Two channels, both built by `.github/workflows/release.yml`.

| Channel | Comes from | Version | GitHub release |
| --- | --- | --- | --- |
| Nightly | every push to `main` | `<Cargo.toml version>-nightly.<run number>` | prerelease |
| Stable | every push to `release/X.Y` | the exact `Cargo.toml` version | normal release |

The version is written in one place: `version` under `[workspace.package]` in the root `Cargo.toml`. The workflow reads the first `version = "..."` line from that file.

## Install and update

Each release has three installers, plus `SHA256SUMS`:

- `Speechify-<version>-windows-x64-installer.exe`
- `Speechify-<version>-mac-arm64.dmg` (Apple silicon)
- `Speechify-<version>-linux-amd64.deb`

To update, install the new version over the old one. Dictations, dictionary, settings and models live in the app data folder and are kept. The installers are not signed. The macOS app is signed ad hoc until there is an Apple Developer ID. There is no in-app updater.

## What a push does

1. **Determine version.** On `release/*` the branch name must be `release/<major>.<minor>` (not three parts) and the `Cargo.toml` version must start with it, or the job fails. On `main` it is the nightly version above. Anything else is a development build and publishes nothing.
2. **Tests.** On pushes, the workflow runs `ci.yml` as a reusable workflow: the tests on Linux, macOS and Windows, and the coverage gate. A release is published only after they pass.
3. **Package.** One job per platform fetches the pinned speech runtime, builds with `SPEECHIFY_VERSION` set, and checks that `speechify --version` prints `speechify <version>`. Linux builds a `.deb`, macOS a signed-ad-hoc `.app` in a `.dmg`, Windows an NSIS installer.
4. **Publish.** It collects the installers, writes `SHA256SUMS`, and creates the release `v<version>` with generated notes. Nightlies are marked as prereleases.

Pull requests run the tests and build every installer, and publish nothing. A commit message containing `[no-release]` on a push skips publishing and keeps the build artifacts (kept for 7 days).

If a release for that tag exists already, the publish step prints a notice and exits without changing it. To ship a fix, bump the version.

## Cutting a stable release

```
git switch main
git switch -c release/0.2                 # branch from main at the commit to ship
# set version = "0.2.0" in the root Cargo.toml, commit, push
git push -u origin release/0.2
```

That builds and publishes `v0.2.0`. Concurrent runs on `release/*` are never cancelled, so no patch release is skipped.

## Patch releases

Fixes ship from the release branch they belong to.

1. Land the fix on `main` first (or cherry-pick it from there).
2. Cherry-pick it onto `release/0.2`.
3. Bump the patch version in `Cargo.toml` (0.2.0 to 0.2.1) in the same push, so the workflow has a new tag to create.


## Dependencies

The policy is in `.github/dependabot.yml`.

- **Security fixes** are proposed and merged as soon as the required checks pass. Cooldown never applies to them.
- **Everything else** waits 14 days after a version is released (`cooldown.default-days: 14` for all semver levels), and minor and patch updates are grouped into one pull request per ecosystem. Dependabot checks npm and Cargo daily and GitHub Actions weekly, so a version is proposed soon after its 14 days are up.
- **Auto-merge** is done by `.github/workflows/dependabot-security.yml`. It looks up whether a Dependabot pull request fixes a security advisory. Patch and minor security fixes get auto-merge with squash once required checks pass. Major security fixes get the `security-major` label and need a same-day review by a person.
- **The secret.** Looking up advisories needs a token the default `GITHUB_TOKEN` does not have. Create a token that can read Dependabot alerts and store it as the repository secret `DEPENDABOT_ALERTS_TOKEN`. Without it the workflow warns and nothing is auto-merged.
- **Overrides.** An npm override or a Cargo `[patch]` must have an entry in `.github/dependency-overrides.yml` in the same commit, and is removed, with its entry, once upstream no longer needs it. The file lists no overrides today.
- **Pins.** Manifests use exact versions, and the workflows pin actions by commit hash. Update them through Dependabot, not by hand.

## Branch protection

The `main` ruleset requires these checks by name: `Test (ubuntu-24.04)`, `Test (windows-2025)` and `Tauri full coverage gate`. If you rename a job in `ci.yml`, update the ruleset in the same change. A cancelled macOS run needs a rerun, not a rewrite.
