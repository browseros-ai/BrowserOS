# bos_build

The build and release system for BrowserOS and BrowserClaw. One Python CLI
(`browseros`) turns a Chromium checkout into signed, packaged browsers, then
ships them.

Run everything from `packages/browseros`:

```bash
cd packages/browseros
uv sync                 # once
cp .env.example .env    # once, then fill in what you need
uv run browseros --help
```

Every `browseros …` command below is really `uv run browseros …`. Drop the
prefix if you have the venv activated.

## Read this first

**`browseros build` builds a binary. It does not release a product.**

A local build produces one browser, one product, one platform, one arch. A
*release* is a GitHub workflow dispatch that builds every platform, uploads to
R2, stages update feeds, and drafts a GitHub release. A human then promotes it
live.

| I want to… | Do this |
| --- | --- |
| Build on my machine | `browseros build --preset debug` |
| See exactly what a build will run | `browseros build --preset release --show-plan` |
| Release BrowserOS or BrowserClaw | `gh workflow run release-browseros.yml` |
| Make a staged release live | `browseros release publish`, then `browseros release appcast --publish` |
| Release an extension CRX | `gh workflow run release-extensions.yml` |
| Update extension feeds | `gh workflow run release-extension-feeds.yml` |
| Grab today's signed mac build | Download the `nightly-browseros` / `nightly-browserclaw` prerelease |
| Check the patch stack | `browseros dev doctor` |

## Mental model

A build is composed, not configured:

```
preset + product + platform + arch + switches  ->  ordered list of steps
```

- **preset** — `release` or `debug`. Owns the shape of the pipeline.
- **product** — `browseros` or `browserclaw`. One file each,
  `products/<id>/product.py`.
- **platform** — taken from the host: macOS, Windows, Linux.
- **arch** — `arm64`, `x64`, or `universal` (macOS only; expands into three
  sequential runs).
- **switches** — flat toggles: `clean`, `provision`, `download`, `sign`,
  `upload`. Resolved CLI > profile > preset default.
  (`bundle_local_extensions` is a switch too, but profile-only — there is no
  CLI flag for it.)

Composition lives in one pure function, `plan()` in `core/planner.py`. Nothing
else decides step order. Steps self-register with `@step(...)` and declare the
env vars they need, so a missing secret fails in preflight — not three hours
into a compile.

Two switches people mix up:

- `--provision` controls the **Chromium checkout** (`none`, `full`, `shallow`).
- `--download` controls whether **server and onboarding resource bundles** are
  pulled from R2. It has nothing to do with Chromium.

### Layout

Three toolsets — BUILD (`steps/` on the `core/` engine), RELEASE (`release/`),
DEV (`patchkit/`) — over shared plumbing (`lib/`) and product data
(`products/`):

```
bos_build/
  browseros.py  entry — the `browseros` Typer app (also `python -m bos_build`)
  cli/          thin Typer wrappers (build, source, product, dev, release, ext, ota)
  core/         engine: context, step registry, planner, runner, pipeline,
                resolver, events, product descriptor model — zero domain knowledge
  lib/          plumbing: env, utils, logger, paths, notify, sparkle, versions, r2
  products/     one package per product: define() call + server bundles
  steps/        BUILD — pipeline steps registered via @step (source, setup,
                resources, patches, extensions, compile, sign, package, storage)
  release/      RELEASE — list, publish, download, github, appcast;
                release/extensions/ packs CRXs, release/feeds/ publishes update
                feeds, release/ota/ ships server OTA updates
  patchkit/     DEV — non-interactive patch surface: extract, batch-apply,
                .features.yaml IO, read-only patch-stack doctor
  profiles/     saved switch sets (flat yaml)
  config/       data: gn flags, resource yamls, appcast templates, build offset
  docs/         the deeper operator docs linked below
```

## Build locally

Always start by looking at the plan. It needs no Chromium checkout:

```bash
browseros build --preset release --show-plan
```

It prints the composed steps and every env var they require, marked set or
missing.

```bash
# Fast iteration.
browseros build --preset debug --chromium-src ~/chromium/src

# Signed local release build, macOS arm64.
browseros build --preset release --product browserclaw --arch arm64

# Release-shaped Windows build against a checkout you already have (one line —
# a Windows path and a shell line-continuation both want the backslash).
browseros build --preset release --provision none --clean --product browserclaw --arch x64 --sign --upload --chromium-src C:\src\chromium-3\src

# Resume after a failure, without recompiling.
browseros build --preset release --from sign_macos

# Subtract steps from the composed plan.
browseros build --preset release --skip upload,series_patches
```

Profiles are saved switch sets in `profiles/`:

| Profile | Used by | What it sets |
| --- | --- | --- |
| `release-ci` | `build-browseros.yml`, the reusable Linux/Windows release lane | `preset: release`, `clean: false`, `provision: none` — the workflow provisions and caches Chromium itself |
| `nightly-ci` | unsigned cloud nightlies | the same, plus `sign: false`, `upload: false` |
| `nightly-macos` | the two signed mac nightlies | `preset: release`, `download: false`, `bundle_local_extensions: true` — build servers and extensions from the working tree |

`release-macos.yml` uses no profile. It runs `--preset release` straight against
the persistent checkout on the self-hosted Mac.

Deeper flag semantics — `--skip`, `--from`, `--gn-arg`, `modules:` profiles,
ephemeral runners — live in [`docs/build-cli.md`](docs/build-cli.md).

## Release a browser

Dispatch-only, one product per run. No tag trigger, no schedule: a release can
never accidentally take WarpBuild or the Mac builder out from under a nightly.

The browser version comes from
`packages/browseros/resources/BROWSEROS_VERSION`. You do not pass it.

```bash
gh workflow run release-browseros.yml \
  -f platforms=all \
  -f include_servers=true \
  -f sign_windows=true \
  -f macos_arch=arm64 \
  -f upload_to_r2=true \
  -f extensions=alpha \
  -f extensions_version=<agent-extension-version> \
  -f github_release_draft=true
```

`release-browserclaw.yml` takes the same inputs; its `extensions_version` is the
BrowserClaw extension version. Dispatch by filename, never by the grouped
`Release:` display name.

Every input has a default, so a bare `gh workflow run release-browseros.yml`
builds all platforms with servers, signed Windows, arm64 mac, alpha extensions,
and a draft release. Narrow it when you don't want all of that:

```bash
gh workflow run release-browseros.yml -f platforms=linux -f extensions=skip
gh workflow run release-browserclaw.yml -f platforms=macos -f macos_arch=universal -f extensions=skip
```

`extensions_version` is required whenever `extensions` is `alpha` or `prod` —
CRX versions are independent of the browser version.

### What CI does, and where it stops

A BrowserClaw run can produce TypeScript claw-server resources, Rust
claw-server resources, browser builds for three platforms, the BrowserClaw
extension CRX, staged update feeds, and draft GitHub release assets.

It **stages**. It does not promote:

- Versioned artifacts land in R2 under the version, not under `download/`.
- Update feeds are rendered as a dry run and uploaded as one Actions artifact,
  `staged-update-feeds-<product>-<version>`.
- The GitHub release is a draft.
- The Actions summary prints the exact promote commands to run.

Going live is a human decision. That is on purpose.

## Promote a release to live

Inspect, then promote. Feed commands are dry runs unless you pass `--publish`.
A publish backs up the live feed to `feeds-history/` first and refuses a version
downgrade (`--allow-downgrade` overrides).

```bash
cd packages/browseros

# 1. See what CI staged.
browseros release list --version <version> --product browseros
browseros release feeds status

# 2. Copy versioned R2 objects to the live download/ aliases.
browseros release publish --version <version> --product browseros

# 3. Diff the appcast, then publish it.
browseros release appcast --version <version> --product browseros
browseros release appcast --version <version> --product browseros --publish
```

Swap in `--product browserclaw` for the other product. If you need to recreate
the draft GitHub release by hand, that is
`browseros release github create --version <version> --draft --product <id>`.
Server OTA promotion is separate, and also manual.

Lane-by-lane detail, required secrets, runner cost, and troubleshooting:
[`docs/release-ci.md`](docs/release-ci.md).

## Release extensions

Four extensions ship as signed CRXs: `agent`, `controller`, `bugreporter`,
`browserclaw`. `agent` and `browserclaw` build from this repo; the other two are
cloned from external repos. All four version independently of the browser.

CRX release and feed updates are separate workflows. Release the binary first:

```bash
gh workflow run release-extensions.yml \
  -f version=0.0.118 \
  -f extension=agent
```

Then inspect a feed dry run or publish it explicitly:

```bash
gh workflow run release-extension-feeds.yml \
  -f channel=alpha \
  -f pins='agent=0.0.118,bugreporter=54.0.0.0'

gh workflow run release-extension-feeds.yml \
  -f channel=alpha \
  -f pins=agent=0.0.118 \
  -f publish=true
```

Pins are optional; extensions not set carry over from the live manifests. The
per-product release orchestrators upload the selected CRX and separately stage
feed previews, but never publish live extension manifests.

Locally there are two commands, and the difference matters:

```bash
# Build, pack, sign, and upload the CRX only.
browseros ext release --version 0.0.118 --name agent

# Feeds only, no CRX build. Pin versions; anything unset carries over from live.
browseros release extensions --channel alpha --set agent=0.0.118
browseros release extensions --channel alpha --set browserclaw=0.1.4 --publish
```

`release extensions` regenerates the update manifest, `extensions.json`, and the
bundled manifest together, so they cannot drift apart.

## Servers and nightlies

Server bundles version independently of the browser, each from its own package
file:

| Bundle | Version source | Workflow | Tag |
| --- | --- | --- | --- |
| BrowserOS agent server | `packages/browseros-agent/apps/server/package.json` | `release-server.yml` | `agent-server/v*` |
| BrowserClaw server (Bun) | `.../apps/claw-server/package.json` | `release-claw-server.yml` | `claw-server/v*` |
| BrowserClaw server (Rust) | `.../apps/claw-server-rust/Cargo.toml` | `release-claw-server-rust.yml` | `claw-server-rust/v*` |

BrowserClaw browser builds download the Bun server today. The Rust blocks in
`config/download_resources.yaml` and `config/copy_resources.yaml` sit beside the
Bun ones, commented out.

Two signed macOS nightlies run on the self-hosted Mac and publish rolling
prereleases anyone can download: `nightly-browseros` (04:00 UTC) and
`nightly-browserclaw` (06:30 UTC). Nightlies build servers and extensions from
the checked-out tree; releases consume published R2 bundles. That contrast is
the point — a nightly tests today's integration, a release ships what R2 already
holds. See [`docs/nightly-macos-ci.md`](docs/nightly-macos-ci.md).

## Patches and products

```bash
browseros dev doctor                            # .features.yaml <-> patches on disk
browseros dev doctor --against ~/chromium/src   # + which patches fail, by feature
browseros dev doctor --feature llm-chat --json  # filtered / machine-readable

browseros product list                          # registered products
browseros product doctor                        # identity uniqueness + branding assets
```

`dev doctor` is read-only, so it runs in CI and before a Chromium bump.
`--against` only ever dry-runs `git apply --check`; the Chromium tree is never
touched. That dry run is stricter than the build's apply step (which falls back
to `--ignore-whitespace` and `--3way`), so a doctor failure means "needs
attention", not necessarily "won't build". Exit 0 healthy, 1 findings, 2 usage
or environment error.

Interactive patch work — `apply`, `extract`, repinning the store to a new
Chromium base — lives in the Rust tool `bpatch`
([`tools/bpatch/README.md`](../tools/bpatch/README.md)). `patchkit/` keeps the
non-interactive Python surface the build steps depend on.

## Where the truth lives

| Thing | Source |
| --- | --- |
| Browser version | `packages/browseros/resources/BROWSEROS_VERSION` |
| Chromium pin | `packages/browseros/CHROMIUM_VERSION`, `BASE_COMMIT` |
| Pipeline shape | `bos_build/core/planner.py` |
| Steps and their required env | `bos_build/steps/`, printed by `--show-plan` |
| Product identity | `bos_build/products/<id>/product.py` |
| Patch stack map | `packages/browseros/chromium_patches/.features.yaml` |
| Which server bundle ships | `config/download_resources.yaml`, `config/copy_resources.yaml` |
| Local secrets | `packages/browseros/.env` (copy `.env.example`) |
| Repo secrets | synced by `tools/release_secrets/sync.py` |

## Deeper docs

| Doc | Read it when |
| --- | --- |
| [`docs/build-cli.md`](docs/build-cli.md) | You need `--skip` / `--from` / `--gn-arg` precedence, `modules:` profiles, or ephemeral-runner setup |
| [`docs/release-ci.md`](docs/release-ci.md) | You are running a release and want the lane map, secrets matrix, and promote commands |
| [`docs/warpbuild-ci.md`](docs/warpbuild-ci.md) | A Linux or Windows cloud build is slow, stuck, or expensive |
| [`docs/nightly-macos-ci.md`](docs/nightly-macos-ci.md) | You are debugging a signed nightly or setting up the Mac builder |
| [`docs/windows-install-verification.md`](docs/windows-install-verification.md) | You are hand-verifying a Windows installer before shipping |

Team-only context lives in the `.internal-docs/` submodule (private; nothing
there is needed to build BrowserOS):

- `setup/release-browser.md` — the operator runbook for one browser release,
  including rollback
- `setup/release-server.md` — publishing server, claw-server, and onboard bundles
- `setup/nightlies.md` — the two mac nightlies and the machine behind them
- `architecture/release-workflows.md` — how the workflows fit together. Older
  than `docs/release-ci.md`; when they disagree, the workflow files win.

## Tests

```bash
uv run python -m unittest discover -s bos_build -t . -p "*_test.py"
uv run ruff check bos_build
```
