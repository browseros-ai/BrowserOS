# BrowserOS Release CI

The BrowserOS and BrowserClaw full-release workflows build one deterministic
release from a fixed default-branch commit. They are dispatch-only because the
browser lanes use paid runners and the dedicated macOS builder.

## Full-release graph

Both products follow the same publication boundary:

```text
preflight
  -> prepare selected components in parallel
  -> snapshot immutable resource pins and the bundled manifest into the release plan
  -> build selected browser platforms with exact component pins
  -> validate selected and skipped platform results
  -> preview every selected update feed, or record a successful no-op
  -> finalize selected components in parallel
  -> write the summary and optionally create the browser GitHub draft
```

BrowserOS prepares the BrowserOS server and agent extension. BrowserClaw
prepares the Rust server, onboarding resources, and BrowserClaw extension.
Preparation uploads immutable versioned objects and leaves private GitHub
drafts. Public component tags/releases and mutable server `latest` aliases are
not created or moved until every selected browser build and the feed preview
succeeds.

The browser GitHub draft is a separate final action. Setting
`github_release_draft=false` skips only that browser draft; selected component
finalizers still run.

## Dispatch rules

A full release must be dispatched from the repository default branch. The
workflow rejects another branch or tag before reserving a component version.
The dispatch event SHA becomes the sole source for component preparation,
release-plan metadata, macOS checkout, browser artifacts, component
finalization, and the browser draft target.

The extension channel defaults to `alpha`. The full workflows do not accept an
extension version: `agent` and `browserclaw` automatically allocate or reuse the
next source-bound four-part version. Standalone extension workflows retain an
explicit version input for repair and exceptional operations.

Normal full releases:

```bash
gh workflow run release-browseros.yml \
  --ref main \
  -f platforms=all \
  -f include_servers=true \
  -f sign_windows=true \
  -f macos_arch=universal \
  -f upload_to_r2=true \
  -f extensions=alpha \
  -f github_release_draft=true

gh workflow run release-browserclaw.yml \
  --ref main \
  -f platforms=all \
  -f include_servers=true \
  -f sign_windows=true \
  -f macos_arch=universal \
  -f upload_to_r2=true \
  -f extensions=alpha \
  -f github_release_draft=true
```

Useful partial runs:

```bash
gh workflow run release-browseros.yml \
  --ref main \
  -f platforms=linux \
  -f include_servers=false \
  -f extensions=skip

gh workflow run release-browserclaw.yml \
  --ref main \
  -f platforms=macos \
  -f include_servers=true \
  -f extensions=skip
```

When `include_servers=false`, the plan snapshots the currently promoted exact
resource versions without preparing or publishing a component release. When
`extensions=skip`, it preserves the complete live extension version set in an
immutable run-scoped manifest without preparing an extension release. Browser
jobs never receive a blank pin that can fall back to a mutable live object.

## Immutable release plan

Before browser jobs start, `bos_build.release.plan` validates every selected
component's version, exact tag, and source SHA. It writes deterministic JSON to
a run/attempt-specific R2 key:

```text
release-plans/<product>/<source-sha>/run-<run-id>-attempt-<attempt>/release-plan.json
```

The schema-v2 plan records the product, browser version, fixed source SHA, run
identity, each selected component version/tag/source, and all three server and
onboarding resource pins. A prepared component supplies its new exact pin. For
every reused family, the helper reads all `latest` targets, resolves one
coherent version, verifies the matching versioned objects, then rereads every
alias to reject a concurrent promotion. Current objects with source-bound R2
metadata use that binding; legacy objects resolve only when exactly one
version's ETag and size match every target. R2 creation uses conditional
`If-None-Match` writes with source, run, plan-key, object-kind, and SHA-256
bindings. A retry may reuse only byte-identical, binding-identical objects.

The helper always reads the complete live bundled manifest, validates its exact
known structure, and verifies every versioned CRX URL. It preserves all pins
when extension release is skipped; otherwise it overrides only the selected
extension. It uploads the result beside the plan:

```text
release-plans/<product>/<source-sha>/run-<run-id>-attempt-<attempt>/bundled-manifest.xml
```

All selected browser platforms receive the same manifest URL and component
versions. The live `extensions/bundled-manifest.xml` is never changed before a
browser build. The plan directory is also uploaded as the retry-unique Actions
artifact `release-plan-<product>-<run-id>-<attempt>`.

GitHub increments the workflow attempt when only failed jobs are rerun. A
successful platform from an earlier attempt and a retried platform may
therefore contribute metadata to the same release. Browser metadata consumers
bind every platform to the fixed source SHA and workflow run ID, but
intentionally do not require one ambient attempt. Plan R2 keys and Actions
artifacts keep the attempt in their identities so rerun outputs remain
separate.

## Exact browser inputs

The full workflows pass these reusable-workflow inputs:

| Product | Exact inputs |
| --- | --- |
| BrowserOS | `browseros_server_version`, `browserclaw_server_version`, `browserclaw_onboard_version`, `bundled_extensions_manifest_url` |
| BrowserClaw | `browseros_server_version`, `browserclaw_server_version`, `browserclaw_onboard_version`, `bundled_extensions_manifest_url` |

Linux and Windows build at the caller's fixed workflow SHA. macOS receives the
same SHA through its explicit `ref` input. A selected component that fails,
cancels, returns blank outputs, reports another SHA, or is unexpectedly skipped
blocks every browser job.

After browser jobs finish, a single gate enforces the platform truth table. A
selected platform must succeed, and every unselected platform must be skipped.
Failure, cancellation, or an unexpected skip prevents feed preview and all
component finalizers.

## Feed previews

Full releases never silently publish client update feeds. They render through
the guarded publisher in dry-run mode and stage files only under the canonical
repository homes:

```text
updates/browser/
updates/extensions/
```

The artifact keeps those exact relative paths and is named
`staged-update-feeds-<product>-<run-id>-<attempt>`. Routine workflow commands do
not use `--allow-downgrade` or `--repair-invalid-live`.

Linux-only with `extensions=skip`, or another selection with no appcast or
extension surface, is a successful no-op. It is distinct from an upstream
failure and does not block component finalization. Preview summaries never
contain actionable publication commands. The final summary emits them only
after every selected browser lane and component finalizer succeeds.

After inspecting the artifact and browser draft, publish selected surfaces
explicitly:

```bash
cd packages/browseros

uv run browseros release publish \
  --version <browser-version> \
  --product <browseros-or-browserclaw> \
  --platform linux --platform win --platform macos \
  --source-sha <fixed-source-sha> \
  --workflow-run-id <run-id>

uv run browseros release appcast \
  --version <browser-version> \
  --product <browseros-or-browserclaw> \
  --platforms all \
  --source-sha <fixed-source-sha> \
  --workflow-run-id <run-id> \
  --publish

uv run browseros release extensions \
  --channel alpha \
  --set <agent-or-browserclaw>=<prepared-extension-version> \
  --publish
```

An alpha extension feed can be inspected in production first, then the same
immutable CRX version can be promoted to `prod`; no rebuild is required.

## Feed ownership and reconciliation

This repository owns all deployed feed snapshots under root `updates/`:

```text
updates/browser/       # BrowserOS and BrowserClaw appcasts
updates/server/        # prod and alpha server OTA appcasts
updates/extensions/    # update manifests, JSON config, bundled manifest
updates/upload.sh      # local menu around the guarded publisher
```

`updates/upload.sh` loads local R2 configuration through `EnvConfig` and
delegates validation, backups, and writes to the guarded publisher. After this
path is deployed and verified in production, remove the old
`api-worker/updates` copies and stop using its uploader so there is only one
feed authority.

The currently deployed alpha manifest is malformed, and the two production
server appcasts contain alpha channel metadata. After this change merges and
before the first default `extensions=alpha` full release, repair those three
live objects once. Otherwise the paid browser builds will finish before the
feed preview fails closed. First review the repair dry run:

```bash
cd packages/browseros
uv run browseros release feeds publish-local \
  extensions/update-manifest.alpha.xml \
  appcast-server.xml \
  appcast-claw-server.xml \
  --repair-invalid-live
```

Publish exactly the reviewed repair, then verify that the ordinary rails pass
without repair authority:

```bash
uv run browseros release feeds publish-local \
  extensions/update-manifest.alpha.xml \
  appcast-server.xml \
  appcast-claw-server.xml \
  --repair-invalid-live \
  --publish

uv run browseros release feeds publish-local \
  extensions/update-manifest.alpha.xml \
  appcast-server.xml \
  appcast-claw-server.xml

uv run browseros release feeds status
```

The repair flag does not authorize a version downgrade; that remains a
separate explicit operation.

## Server OTA policy

Full browser releases pass `publish_ota=false`. A successful server finalizer
publishes its component release and promotes versioned browser resources to the
resource `latest` alias, but it does not publish `updates/server` OTA appcasts.
Server OTA promotion remains an explicit standalone operation with the
appropriate server workflow or OTA command.

The server finalizers also enforce monotonic publication. If a newer version
became public while browsers were building, an older prepared release cannot
move `latest` backward.

## Standalone workflows

| Workflow | Purpose |
| --- | --- |
| `release-server.yml` | BrowserOS server resources and optional server OTA |
| `release-claw-server-rust.yml` | BrowserClaw Rust server resources and optional server OTA |
| `release-claw-onboard.yml` | BrowserClaw onboarding resources |
| `release-extensions.yml` | CRX preparation/finalization without feed publication |
| `release-extension-feeds.yml` | Extension feed preview or explicit publication |
| `release-linux.yml` | Linux browser build |
| `release-windows.yml` | Windows browser build and optional signing |
| `release-macos.yml` | Signed macOS browser build on the persistent builder |

The reusable `workflow_call` interfaces for server-like and extension
workflows expose `mode=build|finalize`. The full orchestrators use deferred
`build` calls to reserve and upload immutable component objects without public
tags or aliases, then call `finalize` with the exact prepared version and
source. Direct server and onboarding dispatches perform the complete lifecycle
and do not accept `mode`; the extension dispatch retains explicit `mode` and
`defer_finalize` controls for standalone repair work.

## Required configuration

Every full run needs `R2_ACCOUNT_ID`, `R2_ACCESS_KEY_ID`,
`R2_SECRET_ACCESS_KEY`, and `R2_BUCKET` because the immutable release plan is
always uploaded, even when browser `upload_to_r2=false`.

Selected BrowserOS server preparation additionally needs
`BROWSEROS_CONFIG_URL`, `POSTHOG_API_KEY`, and `SENTRY_DSN`. BrowserClaw server
preparation needs `CLAW_POSTHOG_KEY`. Selected extension preparation needs its
signing key and associated build-time configuration. Signed Windows lanes need
the eSigner credentials and `SPARKLE_PRIVATE_KEY`. macOS lanes need repository
variables `BROWSEROS_REPO_PATH` and `BROWSEROS_CHROMIUM_SRC` plus the existing
signing/notarization secrets on the persistent builder.

Use `tools/release_secrets/sync.py` to inspect or sync the allowlisted
repository secrets without printing their values:

```bash
tools/release_secrets/sync.py --env-file .env.production --dry-run
tools/release_secrets/sync.py --env-file .env.production --apply
tools/release_secrets/sync.py --check
```

Linux and Windows release builds use WarpBuild runners. macOS uses the
dedicated self-hosted builder. Runner labels, cost expectations, cache behavior,
and queue troubleshooting remain documented in `bos_build/docs/warpbuild-ci.md`.
