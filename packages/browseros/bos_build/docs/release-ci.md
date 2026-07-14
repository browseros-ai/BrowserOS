# BrowserOS Release CI

This document covers the deliberate release workflows. None of these commands
should be run as a smoke test unless the operator intends to spend release
runner time.

## Workflow Map

The primary release entry points are the per-product full-release workflows.
They are dispatch-only and keep BrowserOS and BrowserClaw releases independent:

```text
release-browseros.yml
  preflight
    - read packages/browseros/resources/BROWSEROS_VERSION for browser artifacts
    - require extensions_version when extensions is alpha or prod
    - fail early when selected lane secrets or variables are missing
  server resources, when include_servers=true
    - release-server.yml
  browser builds
    - release-linux.yml -> build-browseros.yml with products=browseros
    - release-windows.yml -> build-browseros.yml with products=browseros
    - release-macos.yml with products=browseros
  extension CRX, when extensions is alpha or prod
    - release-extensions.yml with extension=agent
  stage_updates
    - render appcast and extension feed dry runs from R2 metadata
    - upload staged XML/JSON as staged-update-feeds-browseros-<version>
    - write manual promote commands to the Actions summary
  finalize
    - write the release summary
    - create or refresh draft GitHub release assets when all selected lanes pass

release-browserclaw.yml
  preflight
    - read packages/browseros/resources/BROWSEROS_VERSION for browser artifacts
    - require extensions_version when extensions is alpha or prod
    - fail early when selected lane secrets or variables are missing
  server resources, when include_servers=true
    - release-claw-server.yml
    - release-claw-server-rust.yml
  browser builds
    - release-linux.yml -> build-browseros.yml with products=browserclaw
    - release-windows.yml -> build-browseros.yml with products=browserclaw
    - release-macos.yml with products=browserclaw
  extension CRX, when extensions is alpha or prod
    - release-extensions.yml with extension=browserclaw
  stage_updates
    - upload staged XML/JSON as staged-update-feeds-browserclaw-<version>
  finalize
```

The per-product full-release workflows have no schedule and no tag trigger. A
full release is a manual dispatch so it cannot accidentally occupy WarpBuild or
the self-hosted macOS builder.

## Component Workflows

| Workflow | Purpose | Dispatch | Orchestrator use |
| --- | --- | --- | --- |
| `.github/workflows/release-server.yml` | Builds BrowserOS server resource zips for every browser target, uploads versioned R2 resource keys, attaches server release assets, and reflects the server package version. | Manual and `agent-server/v*` tags | Yes, when `include_servers=true` and `products` includes `browseros` |
| `.github/workflows/release-claw-server.yml` | Builds BrowserClaw server and onboard resource zips, uploads versioned R2 keys, attaches server release assets, and reflects Claw package versions. | Manual and `claw-server/v*` tags | Yes, when `include_servers=true` and `products` includes `browserclaw` |
| `.github/workflows/release-claw-server-rust.yml` | Builds BrowserClaw Rust server resource zips for every browser target, uploads versioned R2 keys under `claw-server-rust/prod-resources`, and attaches Rust server release assets. | Manual, reusable, and `claw-server-rust/v*` tags | Called by `release-browserclaw.yml` when `include_servers=true` |
| `.github/workflows/release-extensions.yml` | Builds, signs, and uploads CRXs for `agent`, `controller`, `bugreporter`, and `browserclaw`; it never reads or writes update feeds. | Manual and reusable | Called by per-product orchestrators with `secrets: inherit` |
| `.github/workflows/release-extension-feeds.yml` | Regenerates extension update feeds through the guarded feed publisher, as a dry run by default or an explicit R2 publish. | Manual and reusable | Independent operator action; no orchestrator dependency |
| `.github/workflows/release-cli.yml` | Builds browseros-cli release binaries, uploads them to CDN, publishes npm package metadata, and creates the CLI GitHub release. | `cli/v*` tags | No orchestrator use |
| `.github/workflows/release-linux.yml` | Builds Linux x64 browser artifacts on WarpBuild, one matrix entry per selected product. | Manual | Yes |
| `.github/workflows/release-windows.yml` | Builds Windows x64 browser artifacts on WarpBuild, one matrix entry per selected product, with optional signing. | Manual | Yes |
| `.github/workflows/release-macos.yml` | Builds signed macOS artifacts on the dedicated self-hosted builder and downloads published server/onboard resource bundles from R2. | Manual | Yes |
| `.github/workflows/release-browseros.yml` | Orchestrates one BrowserOS release, including server resources, selected browser platforms, optional agent CRX upload, staged feed artifacts, and draft GitHub release assets. | Manual only | No reusable entry point |
| `.github/workflows/release-browserclaw.yml` | Orchestrates one BrowserClaw release, including TS and Rust server resources, selected browser platforms, optional BrowserClaw CRX upload, staged feed artifacts, and draft GitHub release assets. | Manual only | No reusable entry point |

Browser artifacts use the BrowserOS browser version from
`packages/browseros/resources/BROWSEROS_VERSION` (for example `0.47.2.2`).
Server resources do not use that version. `release-server.yml` resolves
`packages/browseros-agent/apps/server/package.json` and tags
`agent-server/vX.Y.Z`; `release-claw-server.yml` resolves
`packages/browseros-agent/apps/claw-server/package.json` and tags
`claw-server/vX.Y.Z`; `release-claw-server-rust.yml` resolves
`packages/browseros-agent/apps/claw-server-rust/Cargo.toml` and tags
`claw-server-rust/vX.Y.Z`.

The Rust Claw server lane publishes to a distinct CDN/R2 prefix:
`claw-server-rust/prod-resources/{version,latest}/`. BrowserClaw browser builds
ship the TypeScript/Bun server by default from `claw-server/prod-resources/latest/`.
The Rust download entries in
`packages/browseros/bos_build/config/download_resources.yaml` and the Rust copy
entries in `packages/browseros/bos_build/config/copy_resources.yaml` are kept as
commented alternatives. To ship Rust, uncomment the matching Rust blocks,
comment the Bun blocks, and ensure the Rust workflow has already populated the
matching R2 objects; the bos_build download step fails the whole Chromium build
when a configured key is missing. The Rust copy blocks rename
`browseros-claw-server-rs` to the runtime name `browseros-claw-server`.
BrowserClaw server OTA feeds (`appcast-claw-server*.xml`) remain pinned to the
TypeScript/Bun server bundle until a separate feed migration changes them.

`release-macos.yml` follows this release rule too: it does not build server
resources from the checked-out `packages/browseros-agent` tree. Its browser
build command leaves downloads enabled, so `download_resources` fetches the
published BrowserOS server bundle, the active Bun BrowserClaw server bundle, and
the onboarding bundle from R2 using the runner-local `packages/browseros/.env`
R2 credentials. Flip the commented Rust blocks only when intentionally shipping
the Rust server.

The reusable nesting depth is `release-browseros.yml` or
`release-browserclaw.yml` -> `release-linux.yml` or `release-windows.yml` ->
`build-browseros.yml`, which stays below GitHub's limit of four workflow
levels.

The `bundle_local_extensions` profile switch defaults off for release
reproducibility. Release CI profiles keep it off and consume published extension
bundles. The self-hosted macOS nightly profile sets it true to build and pack
in-repo agent/browserclaw CRXs from the checked-out tree while external required
extensions still come from the bundled CDN manifest. Reusable
`build-browseros.yml` callers enabling such a profile must also pass
`bundle-local-extensions: true` so Bun and extension signing/build env are
prepared.

## Per-Product Full Release Inputs

Use these as the normal release entry points. The extension CRX version is
independent of the browser version; pass `extensions_version` whenever
`extensions` is `alpha` or `prod`.

```bash
gh workflow run release-browseros.yml \
  -f platforms=all \
  -f include_servers=true \
  -f sign_windows=true \
  -f macos_arch=universal \
  -f upload_to_r2=true \
  -f extensions=alpha \
  -f extensions_version=<agent-extension-version> \
  -f github_release_draft=true

gh workflow run release-browserclaw.yml \
  -f platforms=all \
  -f include_servers=true \
  -f sign_windows=true \
  -f macos_arch=universal \
  -f upload_to_r2=true \
  -f extensions=alpha \
  -f extensions_version=<browserclaw-extension-version> \
  -f github_release_draft=true
```

Useful narrower runs:

```bash
# BrowserOS Linux only, still stages feed previews where applicable.
gh workflow run release-browseros.yml \
  -f platforms=linux \
  -f extensions=skip

# BrowserClaw browser artifacts against server resources already staged in R2.
gh workflow run release-browserclaw.yml \
  -f include_servers=false \
  -f extensions=skip

# BrowserClaw macOS universal build only.
gh workflow run release-browserclaw.yml \
  -f platforms=macos \
  -f macos_arch=universal \
  -f extensions=skip
```

Individual workflow examples:

```bash
gh workflow run release-server.yml -f version=0.0.124
gh workflow run release-claw-server.yml -f version=0.0.3
gh workflow run release-claw-server-rust.yml -f version=0.1.0
gh workflow run release-linux.yml -f products=browseros -f upload_to_r2=true
gh workflow run release-windows.yml -f products=browserclaw -f sign=false
gh workflow run release-macos.yml -f products=all   # arch defaults to universal; pass -f arch=arm64 for a faster single-arch build
```

## Secrets And Variables

Repository secret and variable names were checked when this document was
added. Values are never needed locally to inspect this matrix.

Use `tools/release_secrets/sync.py` from the repo root to sync allowlisted
release secrets from the operator's local `.env.production` into repo-level
GitHub secrets:

```bash
tools/release_secrets/sync.py --env-file .env.production --dry-run
tools/release_secrets/sync.py --env-file .env.production --apply
tools/release_secrets/sync.py --check
```

The sync is allowlist-only and keeps values off argv, logs, and temp files by
piping each value to `gh secret set` over stdin. It deliberately excludes local
paths and unrelated API keys from `.env.production`.

| Lane | Required names | Current repo status | Notes |
| --- | --- | --- | --- |
| R2 browser artifacts and final draft release | `R2_ACCOUNT_ID`, `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY`, `R2_BUCKET` | Present | Used by Linux/Windows browser downloads and uploads, server resource uploads, and the draft GitHub release asset step. |
| BrowserOS server resources | R2 names plus `BROWSEROS_CONFIG_URL`, `POSTHOG_API_KEY`, `SENTRY_DSN` | Present | `AGENT_RUNNER_JWT_SECRET` is optional and inlined only when present. Per-product release preflight fails before starting paid builds if selected required names are absent. |
| BrowserClaw server resources | R2 names | Present | `SPARKLE_PRIVATE_KEY` is optional for server OTA publishing; the orchestrator passes `publish_ota=false`. |
| BrowserClaw Rust server resources | R2 names | Present | Uses only GitHub-hosted runners and writes `claw-server-rust/prod-resources`; no signing or OTA secrets are required. |
| Windows signing | `ESIGNER_USERNAME`, `ESIGNER_PASSWORD`, `ESIGNER_TOTP_SECRET`, `SPARKLE_PRIVATE_KEY` | Present after running `tools/release_secrets/sync.py --apply` | `ESIGNER_CREDENTIAL_ID` is optional and is also synced when present. Use `sign_windows=false` only for unsigned verification, not a signed release. |
| Extension releases | R2 names plus `GH_TOKEN`, `BROWSEROS_AGENT_V2_KEY`, `BROWSEROS_CONTROLLER_KEY`, `BUGREPORTER_KEY`, `BROWSERCLAW_KEY`, `POSTHOG_API_KEY`, `VITE_PUBLIC_SENTRY_DSN`, `SENTRY_AUTH_TOKEN`, `SENTRY_ORG`, `SENTRY_PROJECT`, `VITE_PUBLIC_POSTHOG_KEY`, `VITE_PUBLIC_POSTHOG_HOST` | Extension signing, Sentry, and PostHog names are synced by `tools/release_secrets/sync.py`; `GH_TOKEN` is external | `GH_TOKEN` is for private extension repo clones and is not sourced from `.env.production`. |
| macOS release builder | Repository variables `BROWSEROS_REPO_PATH`, `BROWSEROS_CHROMIUM_SRC` | Present | The reusable browser build also reads `MACOS_CERTIFICATE_NAME` and `PROD_MACOS_NOTARIZATION_*` from repo secrets when selected; the certificate P12, certificate password, and keychain password remain external to `.env.production`. |
| GitHub release assets | `GITHUB_TOKEN` | Automatic | Finalize uses it through `GH_TOKEN`. |

## Runner Cost And Time

Linux and Windows release builds use WarpBuild runners. The operational details,
cost ballparks, cache behavior, and stuck-queue troubleshooting live in
`packages/browseros/bos_build/docs/warpbuild-ci.md`; keep that document
as the source of truth for WarpBuild labels and timing expectations.

Rules of thumb:

- Linux and Windows are paid cloud runs and can take several hours.
- The macOS release lane runs on the user's dedicated self-hosted machine and
  can take 6 to 20 hours for all products or universal builds.

## Draft GitHub Release

In per-product workflows, the final job creates draft GitHub release assets only
when every selected server, browser, and selected extension lane succeeds and
`upload_to_r2=true`. It runs one of these commands:

```bash
cd packages/browseros
uv run browseros release github create --version <version> --draft --product browseros
uv run browseros release github create --version <version> --draft --product browserclaw
```

If the target GitHub release already exists and is published, the workflow
refuses to modify it. If it is still a draft, matching product assets are
removed first so reruns refresh the draft from current R2 artifacts.

## Staged Update Feed Artifacts

The per-product workflows stage update-feed files after successful selected
build lanes when `upload_to_r2=true`. The stage job runs dry-run feed commands,
never `--publish` and never `--allow-downgrade`:

```bash
cd packages/browseros
uv run browseros release appcast --version <version> --product <browseros|browserclaw>
uv run browseros release extensions --channel <alpha|prod> --set <agent|browserclaw>=<extension-version>
```

The job uploads the staged XML/JSON files as one artifact:

- `staged-update-feeds-browseros-<version>`
- `staged-update-feeds-browserclaw-<version>`

Appcast rendering is best-effort because the CLI renders the product's full
browser feed set and fails wholesale when a selected platform has no matching
feed artifact or a macOS artifact is missing Sparkle signature metadata. Those
failures are reported as warnings in the Actions summary without failing the
run. Files from a failed feed command are discarded before artifact upload, so
the artifact contains only feed sets whose dry-run command completed.

## Manual Promote To Live

Promotion is deliberately outside CI. Inspect the staged update-feed artifact,
R2 metadata, downloaded artifacts, and draft GitHub release first. Then promote
the product explicitly:

```bash
cd packages/browseros

# Inspect staged browser artifacts.
uv run browseros release list --version <version> --product browseros
uv run browseros release list --version <version> --product browserclaw

# Copy versioned R2 objects to live download/ aliases.
uv run browseros release publish --version <version> --product browseros
uv run browseros release publish --version <version> --product browserclaw

# Publish appcast changes after inspecting the staged artifact.
uv run browseros release appcast --version <version> --product browseros --publish
uv run browseros release appcast --version <version> --product browserclaw --publish

# Publish extension update manifests only if the per-product run built a CRX.
uv run browseros release extensions --channel alpha --set agent=<agent-extension-version> --publish
uv run browseros release extensions --channel alpha --set browserclaw=<browserclaw-extension-version> --publish
```

The extension feed operation is also available as a lightweight workflow that
does not rebuild CRXs. `pins` accepts comma- or space-separated `name=version`
pairs; omit `publish` for the full dry run and diff:

```bash
gh workflow run release-extension-feeds.yml \
  -f channel=alpha \
  -f pins='agent=<agent-extension-version>,bugreporter=<bugreporter-version>'

gh workflow run release-extension-feeds.yml \
  -f channel=alpha \
  -f pins=agent=<agent-extension-version> \
  -f publish=true
```

Server OTA promotion is also manual. The server release workflows can generate
alpha OTA artifacts when their own `publish_ota` input is true, but the
per-product release orchestrators do not enable that input.
