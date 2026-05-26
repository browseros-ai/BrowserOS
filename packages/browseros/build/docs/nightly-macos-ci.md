# Nightly macOS CI

This workflow builds signed BrowserOS macOS arm64 DMGs on the dedicated Mac Mini.
It runs nightly, can be triggered manually from any branch, bumps build versions,
uploads the DMG to the Actions run, and leaves build lifecycle Slack updates to
the existing BrowserOS build notifier.

## What It Builds

Nightly runs execute this command from the persistent build repo clone:

```bash
uv run browseros build --config build/config/release.macos.arm64.yaml --chromium-src "$CHROMIUM_SRC"
```

Manual runs default to the same publishing config:

```bash
uv run browseros build --config build/config/release.macos.arm64.yaml --chromium-src "$CHROMIUM_SRC"
```

Set `upload_to_r2=false` in the manual dispatch form to run an artifact-only
build without publishing to R2.

## One-Time Runner Setup

Register the Mac Mini as a repo-scoped self-hosted runner with the custom
`browseros-builder` label:

```bash
mkdir -p ~/actions-runner
cd ~/actions-runner

./config.sh --url https://github.com/<owner>/<repo> --token <REGISTRATION_TOKEN> \
  --labels browseros-builder --name mac-mini-builder --work _work
```

The workflow targets:

```yaml
runs-on: [self-hosted, macOS, ARM64, browseros-builder]
```

Run the service in the logged-in GUI user session, not as a boot-time daemon.
Codesign and `xcrun notarytool` need access to the user's login keychain; daemon
or SSH-only sessions commonly fail with `User interaction not allowed`.

```bash
./svc.sh install
./svc.sh start
```

If the runner is launched by `launchd`, inject the build toolchain into the
runner PATH and restart the service:

```bash
printf '%s\n' "$HOME/code/depot_tools:/opt/homebrew/bin:/usr/local/bin:$PATH" \
  > ~/actions-runner/.path
./svc.sh stop
./svc.sh start
```

Keep the runner current enough to run the action majors used by the workflow.

## Machine Prerequisites

The Mac Mini must already have:

- Build repo clone, for example `/Users/<user>/code/browseros-release`
- Chromium checkout, for example `/Users/<user>/code/chromium-release/src`
- `uv`, `gh`, depot_tools, Xcode Command Line Tools, and signing/notarization tooling
- `packages/browseros/.env` with signing, notarization, R2, and Slack values
- `MACOS_KEYCHAIN_PASSWORD` in `.env` so the build can unlock the keychain

Do not copy signing, notarization, R2, or Slack secrets into GitHub Actions.
The workflow reuses the machine-local `.env`.

## Repository Variables

Add these in GitHub repo settings under Actions variables:

| Variable | Example | Notes |
| --- | --- | --- |
| `BROWSEROS_REPO_PATH` | `/Users/<user>/code/browseros-release` | Persistent build repo clone. Use an absolute path. |
| `BROWSEROS_CHROMIUM_SRC` | `/Users/<user>/code/chromium-release/src` | Chromium `src` checkout. Use an absolute path. |
| `BROWSEROS_NIGHTLY_REF` | `main` | Optional; falls back to the repo default branch. |

## Version Policy

The workflow calls `build/scripts/bump_version.py`.

- Nightly schedule: `offset+build`, commit and push enabled, R2 upload enabled
- Manual dispatch default: `offset+build`, commit disabled, R2 upload enabled
- Manual hotfix option: choose `offset+patch`
- Manual dry run option: choose `none`

`BROWSEROS_BUILD_OFFSET` is the internal Chromium-build monotonic counter.
`BROWSEROS_BUILD` advances the public nightly semantic version. `BROWSEROS_PATCH`
is reserved for manual hotfix-style builds because setting both build and patch
nonzero produces a four-part version.

Nightly version commits use:

```text
chore(release): build v<VERSION> [skip ci]
```

Version commits are pushed to a `bot/nightly-macos-version-*` branch and opened
as pull requests against the target branch. The workflow tries an immediate
squash merge, then auto-merge, and leaves the PR open if GitHub will not merge it
yet. The persistent clone must already have credentials that can push bot
branches. The workflow's `GITHUB_TOKEN` has `contents: write` and
`pull-requests: write` for the build job so it can create and merge those PRs.

## Manual Branch Build

Open Actions, choose `Nightly macOS Build`, click `Run workflow`, select the
branch in GitHub's native branch picker, then set inputs:

- `bump`: `offset-only`, `offset+build`, `offset+patch`, or `none`
- `commit_version`: commit and push the bumped version files
- `upload_to_r2`: publish to R2/CDN after packaging, enabled by default

Manual dispatch requires approval through the `release-core` environment.
Scheduled nightly runs bypass that approval job and run automatically.

The DMG is always uploaded as a run artifact when packaging succeeds.

## Artifacts

The build writes:

```text
packages/browseros/releases/<version>/BrowserOS_v<version>_arm64.dmg
```

The workflow uploads matching DMGs as `BrowserOS_v<version>_arm64` with
14-day retention.

## Slack

The build already posts pipeline start, phase start/done, success, failure,
package-created, and upload-complete messages when `SLACK_WEBHOOK_URL` is
present in `.env`.

The workflow only adds a CI-level failure ping for failures that happen before
or around the build invocation, such as missing runner variables or sync errors.

## Troubleshooting

`User interaction not allowed`: run the runner as the logged-in GUI user and
confirm `MACOS_KEYCHAIN_PASSWORD` is present in `packages/browseros/.env`.

`uv`, `gclient`, `gn`, or `autoninja` not found: update `~/actions-runner/.path`
and restart the runner service.

Artifact-only manual run: set `upload_to_r2=false` to package the DMG without
publishing it to R2.

No version commit: check `commit_version`, the selected bump mode, the
persistent clone's branch push credentials, and any open
`bot/nightly-macos-version-*` PR.

Long runtime: the release pipeline resets the Chromium tree and wipes
`out/Default_arm64`, so multi-hour runs are expected.
