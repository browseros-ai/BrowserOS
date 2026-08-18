# Nightly macOS CI

Two signed arm64 nightlies orchestrate alpha component publication before the
browser build:

| Workflow | Product | Schedule | Rolling prerelease |
| --- | --- | --- | --- |
| `.github/workflows/nightly-browseros.yml` | BrowserOS | `17 4 * * *` | `nightly-browseros` |
| `.github/workflows/nightly-browserclaw.yml` | BrowserOS neo | `47 6 * * *` | `nightly-browserclaw` |

The non-round minutes avoid GitHub's highest scheduled-workflow load at the
start of each hour. GitHub schedules remain best-effort and may be delayed or
dropped during high load. Use a separate watchdog that checks for a fresh
successful run if the nightly has a strict delivery SLA.

## Release graph

Each run verifies that it was triggered from `main`, then serially reserves a
new browser version by committing `BROWSEROS_VERSION` and the build offset to
`main`. The resulting immutable `main` SHA is used for every release stage:

```text
triggering main SHA
  -> reserve the next browser version on main
  -> publish product server release, latest resources, and alpha OTA
  -> publish product extension release and alpha/bundled feeds
  -> build, sign, upload, and publish the arm64 browser nightly
```

BrowserOS calls `release-server.yml` and releases the `agent` extension.
BrowserOS neo calls `release-claw-server.yml` and releases the `browserclaw`
extension. The extension stage cannot start until the server workflow succeeds,
and the browser cannot start until both component workflows succeed.

The component workflows allocate versions, publish immutable assets, update
their `latest` aliases, publish alpha feeds, and reflect successful versions on
main. Their exact output versions are passed into the browser build, so a later
release cannot change what a queued Mac job consumes. A failed server or
extension publication prevents the browser build.

Tracked server and extension alpha snapshots reach protected `main` through
serialized short-lived pull requests. The exact merged snapshots are published
to R2 only after the merge succeeds.

## Main-only contract

Scheduled workflows run from the latest commit on the repository default
branch. The preflight additionally requires the default branch and triggering
ref to both be `main` and checks that the checkout is exactly `github.sha`.
Version reservation verifies that trigger SHA is still an ancestor of `main`,
then returns the merged reservation SHA and onboarding version used by every
later stage.

Manual runs must also be dispatched from `main`; selecting another branch
fails in preflight. There is no repository-variable override for the nightly
source.

```bash
gh workflow run nightly-browseros.yml \
  --ref main \
  -f upload_to_r2=false

gh workflow run nightly-browserclaw.yml \
  --ref main \
  -f upload_to_r2=false
```

## Published-resource build

Nightlies use the normal published-resource provider:

```bash
cd packages/browseros
uv run browseros build \
  --profile nightly-macos \
  --product <browseros-or-browserclaw> \
  --arch arm64 \
  --resource-mode published \
  --chromium-src "$CHROMIUM_SRC"
```

The profile is:

```yaml
preset: release
resource_mode: published
```

The browser downloads the exact server and onboarding versions plus the exact
product extension version published by the preceding jobs. The reserved
checkout's bundled manifest provides the unchanged extension pins, including
the bug reporter. It does not build Bun, Rust, onboarding, or extension
resources on the Mac. This removes runner-local component toolchains, mutable
`latest` races, and dirty checkout state from the nightly resource contract.

Every successful build uploads its DMG as a 14-day Actions artifact, uploads
browser deliverables to R2 unless disabled for a manual run, and replaces the
product's rolling prerelease with the new DMG.

## Browser version policy

Every scheduled and manual nightly reserves `offset+build` on `main` through a
short-lived pull request before component publication starts. The reservation
serializes both products and retries if another writer moves `main`, so every
run gets a distinct browser version and build offset:

```text
packages/browseros/resources/BROWSEROS_VERSION
packages/browseros/bos_build/config/BROWSEROS_BUILD_OFFSET
```

The build reads the reserved version without changing it and fails if it does
not match the reservation output. A downstream failure therefore leaves a
reserved but unpublished version on `main`; the next nightly advances again
rather than reusing or overwriting it.

## Mac runner boundary

Only the final browser job requires the repository-scoped runner labels:

```yaml
runs-on: [self-hosted, macOS, ARM64, browseros-builder]
```

Server and extension jobs run before that job on their own hosted runners. An
offline Mac therefore does not prevent alpha component publication, but it
does prevent the signed DMG from being created. GitHub leaves an unmatched
self-hosted job queued and eventually cancels it after 24 hours.

Only the final browser job joins the shared `macos-build` concurrency group.
`queue: max` retains up to 100 pending browser jobs in FIFO order instead of
letting a newer run replace an older pending one. Server and extension jobs run
before the Mac lock is requested. Full releases and both nightlies share this
lock.

Run the Mac runner in the logged-in GUI user's session. Codesign and
`xcrun notarytool` need that user's keychain; daemon or SSH-only sessions
commonly fail with `User interaction not allowed`.

The machine needs:

- A persistent BrowserOS checkout.
- A persistent pristine Chromium `src` checkout at the repository pin, used
  only as the APFS clone base.
- `uv`, `gh`, depot_tools, Xcode tools, and Chrome.
- The macOS signing identity and notarization credentials.
- Enough disk for Chromium outputs and DMGs.

Set these repository variables:

| Variable | Meaning |
| --- | --- |
| `BROWSEROS_REPO_PATH` | Absolute path to the persistent BrowserOS checkout |
| `BROWSEROS_CHROMIUM_SRC` | Absolute path to the warm pristine Chromium base `src` |

Component, R2, signing, and notarization credentials come from GitHub Actions
secrets. `SLACK_WEBHOOK_URL` is optional and receives failures after the Mac job
has started.

Before each signed browser build, `.github/scripts/macos-chromium-workspace.sh`
validates the base checkout and creates a run/attempt-specific APFS
copy-on-write clone of the whole gclient root under
`../browseros-ci-apfs-workspaces/`. `bos_build` receives the clone's `src`, so
cleaning, patching, compiling, signing, packaging, and universal merge outputs
stay inside the disposable workspace. Cleanup runs with `if: always()`, and the
next setup reaps abandoned owned workspaces from killed jobs.

## Troubleshooting

No server or extension jobs: confirm the run reached `Freeze main nightly` and
was triggered from `main`. A non-main manual dispatch is rejected deliberately.

Browser job queued with no steps: bring an online runner with all four required
labels into the repository. Component publication may already have succeeded.

Published resource download fails: inspect the preceding server and extension
jobs and verify their versioned objects, `latest` aliases, alpha OTA, and bundled
extension manifest were published.

`User interaction not allowed`: run the runner as the logged-in GUI user and
verify `MACOS_KEYCHAIN_PASSWORD` and the signing identity.

APFS workspace setup fails: confirm the base checkout is on APFS, the helper can
create a same-volume `browseros-ci-apfs-workspaces` sibling directory, the base
`src` is at `packages/browseros/CHROMIUM_VERSION`, and the base has no
BrowserOS output directories or tracked patch/resource changes.

No browser version commit: inspect the hosted `Reserve new browser version on
main` job. It requires `contents: write` and `pull-requests: write`, plus branch
rules that allow the Actions bot to merge a zero-review squash PR. A policy
rejection fails the nightly before any component is published.

Artifact-only browser build: set `upload_to_r2=false`. Component alpha
publication and the rolling GitHub prerelease still occur.
