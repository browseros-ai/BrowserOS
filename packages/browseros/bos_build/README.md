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
| Release an extension CRX to alpha | `gh workflow run release-extensions.yml` |
| Preview or promote extension feeds | `gh workflow run release-extension-feeds.yml` |
| Repair a tracked browser/server appcast | `gh workflow run repair-update-feed.yml` |
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
- **switches** — flat choices such as `clean`, `provision`, `resource_mode`,
  `sign`, and `upload`. Resolved CLI > profile > preset default.

Composition lives in one pure function, `plan()` in `core/planner.py`. Nothing
else decides step order. Steps self-register with `@step(...)` and declare the
env vars they need, so a missing secret fails in preflight — not three hours
into a compile.

Two choices people mix up:

- `--provision` controls the **Chromium checkout** (`none`, `full`, `shallow`).
- `--resource-mode published` downloads released component resources from
  R2/CDN. `--resource-mode source` builds the selected extension, onboarding,
  and native server from the checkout. Neither controls Chromium provisioning.

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

# Source-built integration lane without signing or upload.
browseros build --preset release --product browseros --arch arm64 \
  --resource-mode source --source-sha "$(git rev-parse HEAD)" \
  --no-sign --no-upload --chromium-src ~/chromium/src

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
| `release-ci` | `build-browseros.yml`, the reusable Linux/Windows lane | `preset: release`, `clean: false`, `provision: none` — the workflow provisions and caches Chromium itself |
| `nightly-ci` | unsigned cloud nightlies | the same, plus `sign: false`, `upload: false` |
| `nightly-macos` | both products in the signed family nightly | `preset: release`, `resource_mode: published` |

`release-macos.yml` runs `--preset release` against the persistent checkout on
the self-hosted Mac and receives source or published mode from its caller.

Deeper flag semantics — `--skip`, `--from`, `--gn-arg`, `modules:` profiles,
ephemeral runners — live in [`docs/build-cli.md`](docs/build-cli.md).

## Release a browser

Full releases are dispatch-only and fixed-shape: Linux x64, signed Windows x64,
and signed macOS arm64, x64, and universal. Dispatch from the default branch:

```bash
gh workflow run release-browseros.yml --ref main
gh workflow run release-browserclaw.yml --ref main
```

There are no platform, component, extension-channel, or signing inputs. The
workflow freezes the dispatch SHA, publishes the product server and extension
to alpha in strict order, and passes their exact output versions into every
native browser lane. The committed onboarding version is pinned too, so queued
builds cannot drift to a later component release.

### What CI does, and where it stops

A full run publishes the product server release, latest resource alias, alpha
server OTA, product extension CRX, and alpha/bundled extension feeds before it
builds browser artifacts for the complete native matrix.

The browser release itself remains staged:

- Browser deliverables and metadata land in R2.
- The GitHub release is a draft.
- The production browser appcast is untouched.

Promoting the browser to production is a human decision.

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
Server OTA publication stays separate from a full browser release. A bare
standalone server workflow publishes its alpha appcast; production still
requires an explicit `browseros ota server promote --product <id> --publish`.

Lane-by-lane detail, required secrets, runner cost, and troubleshooting:
[`docs/release-ci.md`](docs/release-ci.md).

## Release extensions

Four extensions ship as signed CRXs: `agent`, `controller`, `bugreporter`,
`browserclaw`. `agent` and `browserclaw` build from this repo; the other two are
cloned from external repos. All four version independently of the browser.

The standalone workflow owns the default alpha lifecycle: for the in-repo
`agent` and `browserclaw` extensions it allocates the next version when
`version` is omitted, builds and verifies the immutable CRX, publishes the
GitHub release, merges the coherent tracked alpha snapshots through a
short-lived pull request, and uploads those exact feed files to R2. External
`controller` and `bugreporter` releases require an explicit version because
their source commit is not the monorepo release SHA.

```bash
gh workflow run release-extensions.yml \
  -f extension=browserclaw

gh workflow run release-extensions.yml \
  -f version=0.1.10.0 \
  -f extension=browserclaw
```

The tracked commit updates `update-manifest.alpha.xml`,
`extensions.alpha.json`, and `bundled-manifest.xml` together. `controller` still
releases a CRX but has no alpha entry because it is not registered in the client
update feed. Selecting `all` requires one explicit version shared by all four
extensions. A deferred build leaves its draft private; its later `finalize`
dispatch performs the alpha update.

Use the feed workflow for previews, repairs, or explicit production promotion:

```bash
gh workflow run release-extension-feeds.yml \
  -f channel=prod \
  -f pins=browserclaw=0.1.10.0

gh workflow run release-extension-feeds.yml \
  -f channel=prod \
  -f pins=browserclaw=0.1.10.0 \
  -f publish=true
```

Pins are optional; extensions not set carry over from the live manifests. The
family nightly renders both in-repository extension pins together and publishes
them only after its one tracked-state squash merge; the standalone extension
workflow remains the independent automatic alpha entrypoint.
With `publish=true`, the feed workflow merges each channel's exact generated
snapshots into the default branch before it uploads those same files to R2. A
failed snapshot merge therefore leaves that channel's live feeds untouched. For
`channel=both`, alpha completes before production so production sees alpha's
newer bundled versions. If production later fails, alpha remains durably
committed and published; rerun the workflow to resume production.

To republish one tracked browser or server appcast through the same validation,
backup, and downgrade guards:

```bash
gh workflow run repair-update-feed.yml \
  -f feed=appcast-server.xml \
  -f repair_invalid_live=true \
  -f publish=true
```

Leave `publish` unchecked for a full dry-run diff. Enable
`repair_invalid_live` only when the live object is malformed or carries the
wrong channel metadata; the repair path still refuses a recoverable downgrade.

Locally there are two commands, and the difference matters:

```bash
# Build, pack, sign, and upload the CRX only.
browseros ext release --version 0.0.118 --name agent

# Feeds only, no CRX build. Pin versions; anything unset carries over from live.
browseros release extensions --channel alpha --set agent=0.0.118
browseros release extensions --channel alpha --set browserclaw=0.1.4 --publish
```

`release extensions` regenerates the update manifest, `extensions.json`, and the
bundled manifest together, so they cannot drift apart. Local `--publish` is an
emergency escape hatch and does not persist `updates/` through git; use the feed
workflow for normal publication.

## Servers and nightlies

Server and onboarding bundles version independently of the browser, each from
its own package file:

| Bundle | Version source | Workflow | Tag |
| --- | --- | --- | --- |
| BrowserOS agent server | `packages/browseros-agent/apps/server/package.json` | `release-server.yml` | `agent-server/v*` |
| BrowserClaw server | `.../apps/claw-server-rust/Cargo.toml` | `release-claw-server.yml` | `claw-server-rust/v*` |
| BrowserOS onboarding | `.../apps/app-onboard/package.json` | `release-app-onboard.yml` | `app-onboard/v*` |
| BrowserClaw onboarding | `.../apps/claw-onboard/package.json` | `release-claw-onboard.yml` | `claw-onboard/v*` |

BrowserClaw browser builds and server OTA both consume the server bundles
published under the historical `claw-server-rust/prod-resources` key. Packaging
normalizes the binary name to `browseros-claw-server` for browser compatibility.

One dispatch-only family workflow builds both signed macOS nightlies on the
self-hosted Mac and publishes the `nightly-browseros` and
`nightly-browserclaw` rolling prereleases. It freezes one source SHA, reserves
one shared browser version in a draft transaction PR, prepares exact private
component resources, and builds both products before public component
finalization. Five tracked server/extension snapshots then reach `main` in one
exact-head squash commit. See
[`docs/nightly-macos-ci.md`](docs/nightly-macos-ci.md).

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
| Which source resources ship | `products/resource_sources.py`, `release/server_resources.py`, `config/copy_resources.yaml` |
| Published-resource compatibility | `config/download_resources.yaml` |
| Family release transaction identity and state | `release/suite.py` |
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
