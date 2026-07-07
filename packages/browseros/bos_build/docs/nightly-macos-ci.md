# Nightly macOS CI

Signed macOS nightlies are two self-hosted arm64 workflows:

| Workflow | Product | Schedule | Version policy | Rolling prerelease |
| --- | --- | --- | --- | --- |
| `.github/workflows/nightly-browseros.yml` | BrowserOS | `0 4 * * *` | Scheduled runs bump `offset+build`, commit through the bot PR flow, and merge with `[skip ci]`. | `nightly-browseros` |
| `.github/workflows/nightly-browserclaw.yml` | BrowserClaw | `30 6 * * *` | Always builds the current version on the selected ref; it does not commit version files. | `nightly-browserclaw` |

Both workflows share the `macos-build` concurrency group, so only one signed
macOS build runs on the Mac Mini at a time. The old
`.github/workflows/nightly-macos-build.yml` workflow has been retired.

## What They Build

Both nightlies build tip-of-tree resources from the persistent checkout and use
the signed nightly profile:

```bash
uv run browseros build --profile nightly-macos --product <product> --arch arm64 \
  --chromium-src "$CHROMIUM_SRC"
```

The profile is `packages/browseros/bos_build/profiles/nightly-macos.yaml`:

```yaml
preset: release
download: false
bundle_local_extensions: true
```

Release-preset defaults still apply for clean, provisioning, signing, package,
Sparkle signing, and upload. Set `upload_to_r2=false` in a manual dispatch to
add `--no-upload` and keep the build artifact-only.

## Local Resource Staging

The nightly profile disables R2 resource downloads because nightly builds are
intended to test the current integration from the checked-out source tree.

BrowserOS stages only the resources used by BrowserOS:

```bash
bun scripts/build/server.ts --target=darwin-arm64 --ci
bun scripts/build/claw-onboard.ts --ci
```

The workflow extracts those artifact zips through
`bos_build.steps.storage.download.extract_artifact_zip` into:

```text
packages/browseros/resources/binaries/browseros_server/darwin-arm64
packages/browseros/resources/binaries/browseros_claw_onboard
```

BrowserClaw stages the shared BrowserOS server bundle, the product-independent
onboarding bundle, and both Claw server variants. The workflow always builds the
TypeScript/Bun bundle:

```bash
bun scripts/build/claw-server.ts --target=darwin-arm64 --ci
```

and extracts it into
`resources/binaries/browseros_claw_server/darwin-arm64`. It also runs:

```bash
packages/browseros-agent/scripts/build/claw-server-rust-local.sh \
  --target darwin-arm64 \
  --agent-root packages/browseros-agent \
  --browseros-root packages/browseros
```

That helper builds the Rust server natively with Cargo and stages:

```text
packages/browseros/resources/binaries/browseros_claw_server_rust/darwin-arm64/
  artifact-metadata.json
  resources/bin/browseros-claw-server-rs
```

The normal resources step then copies this root into Chromium and renames
`browseros-claw-server-rs` to the runtime name `browseros-claw-server`.
Switching the embedded variant is a `copy_resources.yaml` comment swap; the
Rust block is active by default.

## Bundled Extensions

`bundle_local_extensions: true` makes the `bundled_extensions` step build
in-repo required extensions from the checkout while external required
extensions still come from the CDN manifest. The build system loads
`packages/browseros/.env` on import, so the runner-local PEM values such as
`BROWSEROS_AGENT_V2_KEY` and `BROWSERCLAW_KEY` do not need to be exported in the
workflow.

Chrome must be installed on the Mac Mini because CRX packing resolves a Chrome
binary locally.

## Release macOS Workflow

`release-macos.yml` uses the same private Mac Mini, signing keychain, local
`packages/browseros/.env`, and Chromium checkout. Unlike nightlies, releases do
not build tip-of-tree server bundles. They run the normal release preset and let
`download_resources` fetch the published R2 bundles:

```bash
uv run browseros build --preset release --product <product> --arch <arch> \
  --chromium-src "$CHROMIUM_SRC"
```

For BrowserClaw, `download_resources` fetches both Claw server bundles. Rust
resources come from `claw-server-rust/prod-resources/latest/`, and the
TypeScript/Bun resources come from `claw-server/prod-resources/latest/`.

Release runs default to rebuilding the current version files without bumping
them:

```text
bump=none
commit_version=false
upload_to_r2=true
products=browseros
arch=arm64
```

Use `products=browserclaw` to build only BrowserClaw, or `products=all` to
build BrowserOS first and BrowserClaw second in the same job. The workflow also
accepts `arch=universal`; universal and two-product runs use a longer timeout
because they run multiple Chromium build/package passes sequentially.

## One-Time Runner Setup

Register the Mac Mini as a repo-scoped self-hosted runner with the custom
`browseros-builder` label:

```bash
mkdir -p ~/actions-runner
cd ~/actions-runner

./config.sh --url https://github.com/<owner>/<repo> --token <REGISTRATION_TOKEN> \
  --labels browseros-builder --name mac-mini-builder --work _work
```

The workflows target:

```yaml
runs-on: [self-hosted, macOS, ARM64, browseros-builder]
```

Run the service in the logged-in GUI user session, not as a boot-time daemon.
Codesign and `xcrun notarytool` need access to the user's login keychain;
daemon or SSH-only sessions commonly fail with `User interaction not allowed`.

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

Keep the runner current enough to run the action majors used by the workflows.

## Machine Prerequisites

The Mac Mini must already have:

- Build repo clone, for example `/Users/<user>/code/browseros-release`
- Chromium checkout, for example `/Users/<user>/code/chromium-release/src`
- `uv`, `gh`, `bun`, depot_tools, Xcode Command Line Tools, and signing/notarization tooling
- Homebrew Cargo available on PATH for the BrowserClaw Rust nightly
- Chrome installed for local CRX packing
- `packages/browseros/.env` with signing, notarization, R2, Slack, and extension PEM values
- `MACOS_KEYCHAIN_PASSWORD` in `.env` so the build can unlock the keychain

Do not copy signing, notarization, R2, Slack, or extension PEM secrets into
GitHub Actions for the self-hosted macOS nightlies. The workflows reuse the
machine-local `.env`.

## Repository Variables

Add these in GitHub repo settings under Actions variables:

| Variable | Example | Notes |
| --- | --- | --- |
| `BROWSEROS_REPO_PATH` | `/Users/<user>/code/browseros-release` | Persistent build repo clone. Use an absolute path. |
| `BROWSEROS_CHROMIUM_SRC` | `/Users/<user>/code/chromium-release/src` | Chromium `src` checkout. Use an absolute path. |
| `BROWSEROS_NIGHTLY_REF` | `main` | Optional; falls back to the repo default branch. |

## Version Policy

Only the BrowserOS nightly calls `bos_build/scripts/bump_version.py` with a
mutable bump mode.

- BrowserOS schedule: 04:00 UTC, `offset+build`, commit and push enabled, R2 upload enabled
- BrowserOS manual default: `offset+build`, commit disabled, R2 upload enabled
- BrowserOS manual hotfix option: choose `offset+patch`
- BrowserOS manual dry run option: choose `none`
- BrowserClaw schedule and manual runs: `none`; no version commit machinery

04:00 UTC is 9 PM US Pacific during daylight saving time. 06:30 UTC is 11:30 PM
US Pacific during daylight saving time. GitHub cron schedules are UTC-only and
do not track daylight saving changes.

`BROWSEROS_BUILD_OFFSET` is the internal Chromium-build monotonic counter.
`BROWSEROS_BUILD` advances the public nightly semantic version. `BROWSEROS_PATCH`
is reserved for manual hotfix-style builds because setting both build and patch
nonzero produces a four-part version.

BrowserOS nightly version commits use:

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

Open Actions, choose the product workflow, click `Run workflow`, select the
branch in GitHub's native branch picker, then set inputs:

- BrowserOS: `bump`, `commit_version`, and `upload_to_r2`
- BrowserClaw: `upload_to_r2`

The DMG is always uploaded as a run artifact when packaging succeeds. Successful
builds also refresh the product's rolling prerelease tag.

## Artifacts

The builds write:

```text
packages/browseros/releases/<version>/BrowserOS_v<version>_arm64.dmg
packages/browseros/releases/<version>/BrowserClaw_v<version>_arm64.dmg
```

The workflows upload matching DMGs with 14-day retention and refresh:

```text
nightly-browseros
nightly-browserclaw
```

Both GitHub releases are rolling prereleases created with `--latest=false`.

## Slack

When `SLACK_WEBHOOK_URL` is present in `.env`, the build posts one terse phase
narrative for each run. The first message announces the product, version,
OS/arch, and planned phases. Each later phase transition posts one humanized
duration message. The terminal message is always sent synchronously: success
includes R2 artifact links when upload ran, failure names the failing step and
error, and interrupt names the interrupted step. With no webhook configured,
Slack notification is a silent no-op.

The workflows only add a CI-level failure ping for failures that happen before
or around the build invocation, such as missing runner variables or sync errors.

## Troubleshooting

`User interaction not allowed`: run the runner as the logged-in GUI user and
confirm `MACOS_KEYCHAIN_PASSWORD` is present in `packages/browseros/.env`.

`uv`, `gclient`, `gn`, `autoninja`, `bun`, `cargo`, or `chrome` not found:
update `~/actions-runner/.path` and restart the runner service.

Artifact-only manual run: set `upload_to_r2=false` to package the DMG without
publishing it to R2.

No BrowserOS version commit: check `commit_version`, the selected bump mode,
the persistent clone's branch push credentials, and any open
`bot/nightly-macos-version-*` PR.

Long runtime: the release pipeline resets the Chromium tree and wipes
`out/Default_*`, so multi-hour runs are expected.
