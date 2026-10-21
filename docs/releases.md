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

```powershell
# Stable (the latest release that isn't a prerelease)
irm https://github.com/Sumedhbhat/speechify/releases/latest/download/install.ps1 | iex

# Nightly
$env:SPEECHIFY_CHANNEL = "nightly"; irm https://raw.githubusercontent.com/Sumedhbhat/speechify/main/scripts/install.ps1 | iex

# A specific version
$env:SPEECHIFY_CHANNEL = "v0.1.2"; irm https://raw.githubusercontent.com/Sumedhbhat/speechify/main/scripts/install.ps1 | iex
```

Updating is the same command. The installer replaces the app in
`%LOCALAPPDATA%\Programs\Speechify` and never touches
`%APPDATA%\speechify`, where your recordings, corrections, settings, model
environments and fine-tuned models live.

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
| `speechify-<version>.zip` | Source, `uv.lock` and a `VERSION` file. The installer runs `uv sync --frozen` on it, so you get exactly the locked dependencies. |
| `install.ps1` | The installer |
| `SHA256SUMS` | Checksums; the installer verifies the zip against it |

Speech models aren't in the zip. Each one is downloaded the first time you pick
it (see "Models" in the README).

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
2. **Bundle**: runs the tests and builds the zip with the full version stamped
   into it.
3. **Verify**: unpacks the zip on Windows and Linux, runs `uv sync --frozen`,
   and checks `speechify --version` prints the stamped version.
4. **Publish** (pushes only, never PRs): creates the GitHub release with the
   zip, installer and `SHA256SUMS`. Nightlies are marked as prereleases. If a
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
(the `selftest` job in `release.yml`). It runs the real app with a recorded
clip standing in for the microphone and checks, with a screenshot per step:

- holding Right Ctrl transcribes the clip and pastes it into Notepad
- the overlay never takes focus from the app you're dictating into
- Ctrl+Alt+F opens the fix window, a pasted fix replaces the text, and
  Ctrl+Enter learns it and swaps it in Notepad
- the next dictation applies the learned fix
- Ctrl+Alt+H lists both recordings, and Ctrl+Alt+M lists every model

The report and screenshots are attached to each run as the `selftest-report`
artifact. The [Models workflow](../.github/workflows/models.yml) separately
installs and runs every model on Windows, weekly and whenever engine code
changes. Runners have no GPU, so GPU speed is the one thing CI can't measure;
run `speechify selftest` on your own machine for that.
