# Browser release CI

The full BrowserOS and BrowserOS neo workflows orchestrate the existing
component release workflows before building the browser. Component workflows
own component versions and publication. Native browser lanes consume the
normal published-resource manifests and `latest` aliases.

Signed nightlies use independent product transactions through
`nightly-macos-browseros.yml` and `nightly-macos-browserclaw.yml`. Each freezes its
source and resource pins, builds one signed browser, and merges only its product
state before publishing its immutable browser artifacts and rolling prerelease.
The shared allocation and publication locks protect versions and sibling feed
entries. Historical family transactions remain recoverable; see
[nightly-macos-ci.md](nightly-macos-ci.md) for ownership and cutover details.

The existing `release-browseros.yml` and `release-browserclaw.yml` full-release
entrypoints retain their native lane orchestration. Nightly browser jobs build
the exact reservation commit (the frozen source plus its deterministic version
overlay); later state commits and squash merges are never browser build inputs.

## Full-release graph

Both products use the same fixed sequence:

```text
dispatch from main
  -> publish server release, latest resources, and alpha OTA
  -> reflect the successful server version on main
  -> publish extension release, versioned CRX, and alpha/bundled feeds
  -> reflect the successful extension version on main
  -> Linux x64 ───────┐
     Windows x64 ─────┼─> create or refresh the browser draft
     macOS universal ─┘
```

BrowserOS calls `release-server.yml` and releases the `agent` extension.
BrowserOS neo calls `release-claw-server.yml` and releases the `browserclaw`
extension. Each reusable component workflow must finish successfully before
the next stage starts. A failed component release prevents every native
browser lane from starting.

The full workflow freezes the dispatch SHA and passes it to every component
and browser build. Version reflection is intentionally separate: a component
first allocates and builds its next version from that SHA, publishes its tag,
GitHub release, and canonical R2 objects, and only then opens the version-bump
PR. A failed build or publication never changes the committed component
version.

The native lanes use `resource_mode: published`. This is the same path used by
normal local published-resource builds, but the orchestrator pins the exact
server and product-extension versions returned by its component jobs plus the
onboarding version recorded by the frozen checkout. The unchanged extension
pins come from that checkout's bundled manifest.
This prevents another release from changing mutable `latest` or feed aliases
while a native browser lane waits for a runner.

## Dispatch

A dispatch without inputs is a full release: it publishes the product's server
and extension, updates alpha, builds signed Windows x64 and signed macOS
universal, uploads browser deliverables, and creates a draft browser GitHub
release. Neo builds both Linux x64 and arm64 by default; BrowserOS builds Linux
x64. `release-browseros.yml` has no release-shape inputs.

```bash
gh workflow run release-browseros.yml --ref main
gh workflow run release-browserclaw.yml --ref main
```

`release-browserclaw.yml` can narrow that run. Every input defaults to the full
release, and preflight rejects anything else before allocating a version:

| Input | Values | Effect |
| --- | --- | --- |
| `platforms` | `all` (default), `macos`, `linux`, `windows` | native lanes to build; the others are skipped |
| `macos_arch` | `universal` (default), `arm64` | macOS artifact set; non-default only when macOS is selected |
| `linux_arch` | `all` (default), `x64`, `arm64` | Linux artifact set; default is accepted when Linux is skipped; `arm64` requires Linux |
| `publication` | `alpha` (default), `draft-only` | component publication, see below |

`draft-only` passes `defer_finalize: true` to both component workflows. Each
component still allocates a version from the frozen SHA, uploads its immutable
versioned R2 objects and keeps a private GitHub draft, and the browser lanes pin
those exact versions. Nothing downstream of component finalization runs: no
component tags, server `latest` alias, public component releases, server OTA,
extension alpha/bundled feeds, or version-reflection PRs. The macOS lane also
runs with `notify: false`, which skips its Slack failure ping and blanks
`SLACK_WEBHOOK_URL` so a builder `.env` cannot post. The browser draft is
created as usual from only the selected receipts.

```bash
# Signed macOS arm64 test build with private component drafts.
gh workflow run release-browserclaw.yml --ref main \
  -f platforms=macos -f macos_arch=arm64 -f publication=draft-only

# Cross-compiled Linux arm64 test build with private resource drafts.
gh workflow run release-browserclaw.yml --ref main \
  -f platforms=linux -f linux_arch=arm64 -f publication=draft-only
```

Linux arm64 uses the existing x64 WarpBuild runner and published arm64 server
resources. `linux_arch=all` builds the x64 and arm64 AppImage/DEB pairs serially
so their shared Linux release metadata cannot lose an architecture. The browser
draft checks exactly the selected pairs. Pass the same `--linux-arch` selection
to later release CLI validation/promotion. See [Linux arm64](linux-arm64.md) for
the design, source-mode limit, retry provenance, and native verification gate.

The deferred component drafts remain allocations bound to that source SHA: a
later dispatch of the same SHA reuses them, and releases from other SHAs skip
their versions. Browser R2 objects are versioned but not write-once, so give the
next real release a new browser version instead of reusing a test build's.

The dispatch ref must be the repository default branch. Product, component,
native-lane, and shared Mac concurrency groups use `queue: max`, so up to 100
pending releases wait instead of a newer dispatch replacing an older one.

The browser version comes from `resources/BROWSEROS_VERSION` at the frozen
dispatch SHA. Set that version on main before dispatching when the browser
itself needs a new version. Server and extension versions are independently
allocated by their component workflows.

## Standalone component releases

The component workflows remain independently dispatchable and use main by
default:

```bash
gh workflow run release-server.yml --ref main
gh workflow run release-claw-server.yml --ref main
gh workflow run release-extensions.yml --ref main -f extension=agent
gh workflow run release-extensions.yml --ref main -f extension=browserclaw
```

Direct server dispatches default to `publish_ota=true`. They publish the
versioned and `latest` resources, render all five platform appcast fragments,
merge the tracked snapshot through a short-lived pull request, and then publish
that exact appcast to alpha.

Direct extension dispatches default to `publish_alpha_feed=true`. They publish
the CRX release and canonical versioned R2 object, update the alpha and bundled
manifests atomically through a short-lived pull request, publish those exact
manifests, and reflect in-repository extension versions only after finalization
succeeds.

Production promotion remains explicit:

```bash
cd packages/browseros
uv run browseros ota server promote --product browseros --publish
uv run browseros ota server promote --product browserclaw --publish
```

## Retry procedures

Rerun failed jobs in the original full-release run:

```bash
RUN_ID=<github-run-id>
gh run rerun "$RUN_ID" --failed
gh run watch "$RUN_ID" --exit-status
```

Component allocation and publication are idempotent. A rerun recovers a
matching release for the same source SHA instead of silently allocating a new
version. Successful earlier stages are not rebuilt, and downstream browser
lanes stay gated until the failed stage succeeds.

For the combined signed nightly, rerun failed jobs in the original run when
possible. The stable transaction identity is `nightly-<source-sha>`. Browser
jobs fetch the transaction branch before merge, or
`refs/pull/<PR_NUMBER>/head` after branch deletion, only to make the proven
transaction history reachable. They verify that history contains the recorded
reservation, then always check out `reservation_sha`: the frozen source plus
the exact version/component overlay, without the later tracked-state commit.
They never rebuild from the squash merge SHA, whose tree may include unrelated
`main` commits. A new whole-run invocation that finds the transaction already
merged fails closed; post-merge recovery must rerun failed jobs so it reuses the
successful signed artifacts from the original run.

If browser draft creation alone fails, rerun that job in the original run. It
uses the R2 metadata written by the selected native lanes and verifies the same
source SHA and workflow run ID before refreshing the draft. It runs only when
every selected lane succeeded and every unselected lane was skipped. Native lanes from
different rerun attempts are valid because GitHub keeps the run ID stable.

## Local published-resource build

The full workflows use the standard published-resource path. The equivalent
local build is:

```bash
cd /path/to/BrowserOS/packages/browseros
uv run browseros build \
  --preset release \
  --product browseros \
  --arch x64 \
  --resource-mode published \
  --no-sign \
  --no-upload \
  --chromium-src /path/to/chromium/src
```

Use `--product browserclaw` for BrowserOS neo. Published mode resolves mutable
component aliases, so publish the intended component releases before starting
the browser build.

## Local source build

Source mode remains available for local development. It builds
the product extension, onboarding bundle, and active host's server from the
checkout; only the pinned bug reporter is downloaded. It does not publish
component tags, `latest` aliases, server OTA, or extension feeds.

```bash
cd /path/to/BrowserOS/packages/browseros
SOURCE_SHA="$(git rev-parse HEAD)"

uv run browseros build \
  --preset release \
  --product browseros \
  --arch x64 \
  --resource-mode source \
  --source-sha "$SOURCE_SHA" \
  --no-sign \
  --no-upload \
  --chromium-src /path/to/chromium/src
```

Chrome, Bun, and the product's build-time secrets must be available. BrowserOS
neo additionally needs the native Rust toolchain. Source mode does not bump
versions, commit, push, or open a PR.

## Publication boundary

The full workflow creates or refreshes a draft browser GitHub release after
every selected native lane uploads complete R2 metadata. It does not publish the
browser appcast. Inspect the browser draft before promotion.

| Workflow | Owned publication |
| --- | --- |
| `release-server.yml` | BrowserOS server release, versioned/`latest` resources, alpha OTA, version reflection |
| `release-claw-server.yml` | BrowserOS neo server release, versioned/`latest` resources, alpha OTA, version reflection |
| `release-claw-onboard.yml` | onboarding release and resources |
| `release-extensions.yml` | extension CRX release, versioned object, alpha/bundled manifests, version reflection |
| `release-extension-feeds.yml` | explicit extension manifest preview or publication |
| `nightly-macos-browseros.yml` | BrowserOS nightly transaction, product alpha snapshots, and `nightly-browseros` |
| `nightly-macos-browserclaw.yml` | BrowserOS neo nightly transaction, product alpha snapshots, and `nightly-browserclaw` |
| `release-browseros.yml` | ordered BrowserOS component releases, native builds, browser draft |
| `release-browserclaw.yml` | ordered BrowserOS neo component releases, native builds, browser draft |

## Required configuration

Component publication and browser uploads use the `R2_*` secrets. BrowserOS
server builds need `BROWSEROS_CONFIG_URL`, `POSTHOG_API_KEY`, and `SENTRY_DSN`;
BrowserOS neo server builds need `CLAW_POSTHOG_KEY`. Extension builds need the
matching signing key and build-time secrets.

Windows signing needs the eSigner secrets and `SPARKLE_PRIVATE_KEY`. macOS uses
repository variables `BROWSEROS_REPO_PATH` and `BROWSEROS_CHROMIUM_SRC` plus
the signing and notarization secrets on the persistent builder. Signed macOS
releases require separate base64-encoded Developer ID provisioning profiles:
`PROD_MACOS_BROWSEROS_PASSKEY_PROFILE_B64` for `com.browseros.BrowserOS` and
`PROD_MACOS_BROWSERCLAW_PASSKEY_PROFILE_B64` for
`com.browseros.BrowserClaw`. Each profile must authorize team `8YMKWU47S5`, its
bundle-specific keychain groups, and
`com.apple.developer.web-browser.public-key-credential`; the build validates
those claims before signing. The profiles are not interchangeable because
Apple assigns the managed capability to an exact App ID.
Until Apple approves a profile, its secret may remain unset: the corresponding
browser still builds, signs, and runs with the standard entitlements, but macOS
platform passkeys are unavailable. Once a secret is configured, a missing,
wrong, or malformed profile is a hard release error.
`BROWSEROS_CHROMIUM_SRC` is a dedicated, CI-owned APFS clone base. Setup keeps
its pinned Chromium identity strict but repairs local Git changes and
BrowserOS-owned output directories before the release runs against a disposable
copy-on-write workspace. The workspace is cleaned under `if: always()`. Runner
labels, cache behavior, and queue recovery are documented in `warpbuild-ci.md`;
persistent macOS setup is in `nightly-macos-ci.md`.
