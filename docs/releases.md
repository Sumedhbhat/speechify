# Releases

Two channels, both built by `.github/workflows/release.yml`.

| Channel | Comes from | Version | GitHub release |
| --- | --- | --- | --- |
| Nightly | every push to `main` | `<Cargo.toml version>-nightly.<run number>` | prerelease |
| Stable | every push to `release/X.Y` | the exact `Cargo.toml` version | normal release |

The version is written in one place: `version` under `[workspace.package]` in the root `Cargo.toml`. The workflow reads the first `version = "..."` line from that file.

## Install and update

Each release has three installers, the macOS update archive, and `SHA256SUMS`:

- `Speechify-<version>-windows-x64-installer.exe`
- `Speechify-<version>-mac-arm64.dmg` (Apple silicon)
- `Speechify-<version>-mac-arm64.app.tar.gz` and `.sig` (read by the in-app updater)
- `Speechify-<version>-linux-amd64.deb`

On macOS, Speechify updates itself. Settings › Updates checks when the page opens and offers **Install and restart**. The menu bar's **Check for Updates…** opens that page, and is renamed **Update to <version>…** when a background check (a minute after start, then every six hours) finds one. With Speechify running, `speechify --update` installs the newest release from a terminal. Nothing installs without being asked.

On Windows and Linux, install the new version over the old one. Dictations, dictionary, settings and models live in the app data folder and are kept on every platform.

## macOS signing and updates

macOS remembers Accessibility access together with the app's *designated requirement*. An ad-hoc signature's requirement is the hash of one exact build (`cdhash H"…"`), so every new build counted as a new app and people had to remove and re-add Speechify after each update. Releases are signed with one self-signed certificate instead. The requirement becomes `identifier "org.sumedhbhat.speechify" and certificate root = H"<certificate hash>"`, which every release signed with that certificate meets, so the permission is kept. `scripts/macos-package.sh` checks the requirement and fails if it still pins a hash.

What it does not change: the certificate is not from Apple, so Gatekeeper still asks people to allow the app (System Settings › Privacy & Security › Open Anyway) the first time they open a downloaded `.dmg`, as it did with ad-hoc signing. Updates installed by the app are not quarantined and open without that prompt. Moving to an Apple Developer ID with notarization later would remove the first-open prompt, and would ask everyone to allow Accessibility one more time.

Updates come from a feed on the `updates` release (a prerelease, so it is never shown as the latest download): `stable.json` for stable builds and `nightly.json` for nightlies. The publish job rewrites its channel's feed only after the versioned release exists. Each feed names one `.app.tar.gz`; the app checks it against the public key in `src-tauri/tauri.conf.json` (`plugins.updater.pubkey`) before replacing itself, then restarts. Nightlies compare by run number (`0.1.0-nightly.1000` is newer than `0.1.0-nightly.999`), and a stable `0.1.0` is newer than any `0.1.0-nightly.N`.

### Secrets

Signing runs in its own step after the build, and only on pushes, so dependency build scripts and pull requests never see these. Pull request builds are signed ad hoc and have no update archive. A push to `main` or `release/*` fails without them rather than publishing a build that would make everyone allow Accessibility again.

| Secret | What it is |
| --- | --- |
| `MACOS_CERTIFICATE_P12` | The signing certificate and key as a base64 `.p12` (`base64 -i signing.p12`) |
| `MACOS_CERTIFICATE_PASSWORD` | Its password |
| `TAURI_SIGNING_PRIVATE_KEY` | The update key: the contents of the file `tauri signer generate` writes |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Its password |

Keep offline copies of both keys. **Never replace the certificate**: a release signed with a different one makes every user allow Accessibility again. If the update key is lost, installed apps cannot verify new updates and everyone has to download the next `.dmg` by hand; a new key also needs its public half in `tauri.conf.json`.

To create them, once (the certificate's `[ext]` must allow code signing):

```sh
printf '%s\n' '[req]' 'distinguished_name=dn' 'x509_extensions=ext' 'prompt=no' \
  '[dn]' 'CN=Speechify Release Signing' 'O=Speechify' \
  '[ext]' 'basicConstraints=critical,CA:false' 'keyUsage=critical,digitalSignature' \
  'extendedKeyUsage=critical,codeSigning' 'subjectKeyIdentifier=hash' > cs.cnf
/usr/bin/openssl req -x509 -newkey rsa:3072 -nodes -keyout key.pem -out cert.pem -days 7300 -config cs.cnf
/usr/bin/openssl pkcs12 -export -inkey key.pem -in cert.pem -out signing.p12 -name "Speechify Release Signing"
npx tauri signer generate -w updater.key      # prints the public key for tauri.conf.json
```

## What a push does

1. **Determine version.** On `release/*` the branch name must be `release/<major>.<minor>` (not three parts) and the `Cargo.toml` version must start with it, or the job fails. On `main` it is the nightly version above. Anything else is a development build and publishes nothing.
2. **Tests.** On pushes, the workflow runs `ci.yml` as a reusable workflow: the tests on Linux, macOS and Windows, and the coverage gate. A release is published only after they pass.
3. **Package.** One job per platform fetches the pinned speech runtime, builds with `SPEECHIFY_VERSION` set, and checks that `speechify --version` prints `speechify <version>`. Linux builds a `.deb` and Windows an NSIS installer. macOS builds the `.app` (nightlies point the updater at `nightly.json`), then `scripts/macos-package.sh` signs it, wraps it in a `.dmg` and writes the signed update archive.
4. **Publish.** It collects the installers, writes `SHA256SUMS`, and creates the release `v<version>` with generated notes. Nightlies are marked as prereleases. Then it points the channel's update feed at the new release.

Pull requests run the tests and build every installer, and publish nothing. A commit message containing `[no-release]` on a push skips publishing and keeps the build artifacts (kept for 7 days).

If a release for that tag exists already, the publish step prints a notice and exits without changing it. To ship a fix, bump the version.

## Cutting a stable release

Only one release branch takes changes at a time: the current line. Every other `release/*` branch is read-only, and the release workflow refuses to publish from one. `scripts/cut-release.sh` moves to a new line (see Branch rules below).

1. Set `version = "0.2.0"` in the root `Cargo.toml` and run `cargo update --workspace` (CI builds with `--locked`), then merge that to `main` through a pull request. Nightlies become `0.2.0-nightly.N`.
2. Run `scripts/cut-release.sh 0.2`. It retires the current line (for example `release/0.1`, which becomes read-only), then creates `release/0.2` from `main`. Creating the branch publishes `v0.2.0`.
3. Through another pull request, set `main` to the next minor version (`0.3.0`), so nightlies count toward the next release.

The script refuses a line older than the current one, a line `main`'s version does not match, and a branch that exists already. It needs the repository admin role, because only admins may create `release/*` branches.

## Patch releases

Fixes ship on the current line only. Once `release/0.2` is cut, `release/0.1` takes no more changes, and a fix for 0.1 users ships as 0.2.x.

1. Land the fix on `main` first.
2. Open a pull request into `release/0.2` that cherry-picks it and bumps the patch version (0.2.0 to 0.2.1) with its `Cargo.lock`, so the workflow has a new tag to create and the `--locked` build passes.
3. Merging it publishes `v0.2.1`. Merge one patch release at a time and let it finish before merging the next: GitHub keeps only one pending run per branch.

## Dependencies

The policy is in `.github/dependabot.yml`.

- **Security fixes** are proposed and merged as soon as the required checks pass. Cooldown never applies to them.
- **Everything else** waits 14 days after a version is released (`cooldown.default-days: 14` for all semver levels), and minor and patch updates are grouped into one pull request per ecosystem. Dependabot checks npm and Cargo daily and GitHub Actions weekly, so a version is proposed soon after its 14 days are up.
- **Auto-merge** is done by `.github/workflows/dependabot-security.yml`. It looks up whether a Dependabot pull request fixes a security advisory. Patch and minor security fixes get auto-merge with squash once required checks pass. Major security fixes get the `security-major` label and need a same-day review by a person.
- **The secret.** Looking up advisories needs a token the default `GITHUB_TOKEN` does not have. Create a token that can read Dependabot alerts and store it as the repository secret `DEPENDABOT_ALERTS_TOKEN`. Without it the workflow warns and nothing is auto-merged.
- **Overrides.** An npm override or a Cargo `[patch]` must have an entry in `.github/dependency-overrides.yml` in the same commit, and is removed, with its entry, once upstream no longer needs it. The file lists no overrides today.
- **Pins.** Manifests use exact versions, and the workflows pin actions by commit hash. Update them through Dependabot, not by hand.

## Branch rules

Repository rulesets (Settings › Rules › Rulesets) enforce these. Nobody can bypass them except where noted, the admin included.

| Ruleset | Applies to | What it does |
| --- | --- | --- |
| `main: pull requests only` | `main` | Changes only through pull requests with these checks passing: `Test (ubuntu-24.04)`, `Test (windows-2025)` and `Tauri full coverage gate`. No force pushes or deletion. |
| `release: pull requests only` | `release/*` | The same, for release branches. |
| `release: create` | `release/*` | Only the repository admin role may create release branches. |
| `release: retired` | `release/*` except the current line | Nothing may change these branches, not even through a pull request. `scripts/cut-release.sh` moves its one exclusion to the new line. |

The release workflow checks the rules on a `release/*` branch before publishing and stops if `release: retired` covers it, so a branch created by hand without the script publishes nothing.

If you rename a job in `ci.yml`, update both pull request rulesets in the same change. A cancelled macOS run needs a rerun, not a rewrite.
