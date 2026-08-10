# Browser release CI

The full BrowserOS and BrowserOS neo workflows build one immutable candidate
from source on every supported host. GitHub Actions chooses runners, scopes
secrets, transports artifacts, and enforces job dependencies. `bos_build` owns
version allocation, resource production, browser builds, attestations, the
release gate, and finalization.

## Full-release graph

Both products use the same fixed graph:

```text
dispatch from main
  -> create or recover candidate commit and PR
  -> build common resources once
  -> Linux x64 ───────┐
     Windows x64 ─────┼─> validate complete lane evidence
     macOS universal ─┘
  -> merge the unchanged candidate PR
  -> create or refresh the browser draft and appcast preview
```

The candidate commit bumps the product's server and extension versions:

| Product | Bumped | Rebuilt without a bump |
| --- | --- | --- |
| BrowserOS | Bun server, `agent` | onboarding |
| BrowserOS neo | Rust server, `browserclaw` | onboarding |

The common job builds the product CRX and onboarding bundle, resolves one
pinned bug reporter CRX from the canonical bundled manifest, and records every
file in `prepared-resources.json`. That directory moves between jobs as a
candidate-bound GitHub artifact. It is not published to R2.

Each native lane validates the common manifest, builds only its native server
targets, stages the resources into the existing Chromium layout, runs the full
browser build, uploads browser deliverables, and emits a lane manifest. The
gate requires Linux x64, signed Windows x64, and signed macOS arm64, x64, and
universal outcomes from the same candidate and common-resource digest.

The full workflows do not call standalone component workflows and do not
publish component tags, GitHub releases, `latest` aliases, server appcasts, or
extension feeds. Component OTA remains a separate operation.

## Dispatch

There are no release-shape inputs. A full release always builds the required
matrix, signs Windows and macOS, uploads browser deliverables, and creates a
browser draft plus an appcast preview.

```bash
gh workflow run release-browseros.yml --ref main
gh workflow run release-browserclaw.yml --ref main
```

A new candidate is accepted only when the dispatch ref is the repository
default branch and the checked-out SHA equals the dispatch SHA. Product-level
concurrency is non-cancelling, so one candidate cannot replace another while
paid native lanes are running.

## Candidate identity and recovery

`browseros release candidate ensure` uses a deterministic branch for the
product and parent commit. The first call allocates versions, stamps only the
selected manifests and lockfile entries, creates one commit, pushes the branch,
and opens a PR. Later calls for the same parent verify and return that exact
candidate; they never amend or force-push it.

The workflow stores the candidate record as:

```text
release-candidate-<product>-<candidate-sha>
```

Common resources and gate evidence use the same immutable identity:

```text
release-resources-<product>-<candidate-sha>
release-gate-<product>-<candidate-sha>
release-finalization-<product>-<candidate-sha>
```

To create or recover a candidate outside Actions, start from a clean checkout
of the default branch:

```bash
cd /path/to/BrowserOS
git switch main
git pull --ff-only

REPO="$(gh repo view --json nameWithOwner --jq .nameWithOwner)"
DEFAULT_BRANCH="$(gh repo view --json defaultBranchRef --jq .defaultBranchRef.name)"
PARENT_SHA="$(git rev-parse HEAD)"
CANDIDATE_RECORD="$(mktemp)"

cd packages/browseros
uv run browseros release candidate ensure \
  --product browseros \
  --parent-sha "$PARENT_SHA" \
  --default-branch "$DEFAULT_BRANCH" \
  --dispatch-ref "$DEFAULT_BRANCH" \
  --repo "$REPO" \
  --output "$CANDIDATE_RECORD"
```

Use `--product browserclaw` for BrowserOS neo. Repeating the same command with
the same parent recovers the existing candidate record.

## Local source build

Source mode is the same resource path used by full releases and nightlies. It
builds the product extension, onboarding bundle, and active host's server from
the checkout; only the pinned bug reporter is downloaded. Chrome, Bun, and the
product's build-time secrets must be available. BrowserOS neo additionally
needs a native Rust toolchain.

On a clean checkout, one command prepares the local common directory and runs
the active host lane:

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

Choose the architecture supported by the host. Omit `--no-sign` only when the
host has the required signing identity. Source mode does not bump versions,
commit, push, or open a PR.

When `--prepared-resources` is omitted, the exact common directory is:

```text
packages/browseros/resources/binaries/prepared_common/
  <product>/<source-sha>/<browser-version>/
```

The source SHA and browser version are both part of the path so two local
nightly versions from one commit cannot reuse incompatible resources.

## Build from an exact prepared directory

A candidate record can prepare the platform-independent resources once, then
any compatible host can validate and consume the same directory:

```bash
SOURCE_REPO=/path/to/BrowserOS
CANDIDATE_RECORD=/path/to/candidate.json
REPO_ROOT=/tmp/BrowserOS-candidate
PREPARED_DIR=/path/to/prepared-browseros
CANDIDATE_SHA="$(jq -r .candidate_sha "$CANDIDATE_RECORD")"
CANDIDATE_BRANCH="$(jq -r .branch "$CANDIDATE_RECORD")"

git -C "$SOURCE_REPO" fetch origin "$CANDIDATE_BRANCH"
git -C "$SOURCE_REPO" worktree add --detach "$REPO_ROOT" "$CANDIDATE_SHA"

cd "$REPO_ROOT/packages/browseros"
uv run browseros release resources prepare \
  --candidate "$CANDIDATE_RECORD" \
  --repo-root "$REPO_ROOT" \
  --output "$PREPARED_DIR"

uv run browseros build \
  --profile release-ci \
  --product browseros \
  --arch x64 \
  --resource-mode source \
  --prepared-resources "$PREPARED_DIR" \
  --no-sign \
  --no-upload \
  --chromium-src /path/to/chromium/src
```

`REPO_ROOT` must be checked out at the candidate SHA recorded in
`candidate.json`. A product, source SHA, browser version, component version,
path, size, extension ID, or checksum mismatch fails before Chromium compile.
The command still builds the current host's server locally; prepared common
resources are intentionally platform-independent.

## Host compatibility boundary

One machine cannot produce the complete signed release matrix.

| Host lane | Browser outcomes | Server targets |
| --- | --- | --- |
| Linux x64 | Linux x64 | `linux-x64` |
| Windows x64 | signed Windows x64 | `windows-x64` |
| macOS arm64, universal plan | signed macOS arm64, x64, universal | `darwin-arm64`, `darwin-x64` |

Common resources can be prepared on any supported machine with Bun, Chrome,
network access, and the extension signing secret. The Rust BrowserOS neo server
must be built on the target operating system. Browser signing and notarization
must run on the corresponding configured host.

## Retry procedures

### A lane fails before merge

Rerun only the failed jobs in the same workflow run:

```bash
RUN_ID=<github-run-id>
gh run rerun "$RUN_ID" --failed
gh run watch "$RUN_ID" --exit-status
```

The candidate SHA and artifact names do not change. Successful artifacts remain
available, the common job validates or recovers its candidate-bound artifact,
and the candidate PR stays unmerged until the complete gate passes. Do not
dispatch a new version or run a standalone component release to repair a lane.

### Finalization fails after merge

The merge command is idempotent and the merged candidate record is retained.
Rerun the failed finalization job in the original run:

```bash
RUN_ID=<github-run-id>
gh run rerun "$RUN_ID" --failed
gh run watch "$RUN_ID" --exit-status
```

This refreshes the draft and appcast preview from the existing gate. It does
not allocate versions or rebuild browsers. For a manual recovery, download the
merged candidate and gate artifacts and call the same Python finalizer:

```bash
gh run download "$RUN_ID" \
  --name "release-candidate-$PRODUCT-$CANDIDATE_SHA" \
  --dir /tmp/browser-candidate
gh run download "$RUN_ID" \
  --name "release-gate-$PRODUCT-$CANDIDATE_SHA" \
  --dir /tmp/browser-gate

cd packages/browseros
uv run browseros release browser finalize \
  --candidate /tmp/browser-candidate/candidate.json \
  --gate /tmp/browser-gate/gate.json \
  --repo "$(gh repo view --json nameWithOwner --jq .nameWithOwner)" \
  --preview-dir /tmp/browser-finalization/previews \
  --output /tmp/browser-finalization/finalization.json
```

Set `PRODUCT` to `browseros` or `browserclaw` and `CANDIDATE_SHA` to the value
shown in the workflow summary.

## Intentional published-resource builds

Published mode remains available for one-off consumers that deliberately want
the already released component resources from R2/CDN. It is not the full
browser release path and does not create a candidate:

```bash
gh workflow run release-linux.yml \
  --ref main \
  -f products=browseros \
  -f upload_to_r2=false
```

The equivalent local command is:

```bash
cd packages/browseros
uv run browseros build \
  --preset release \
  --product browseros \
  --arch x64 \
  --resource-mode published \
  --no-sign \
  --no-upload \
  --chromium-src /path/to/chromium/src
```

Published mode uses `config/download_resources.yaml` and may resolve mutable
component aliases. Never substitute it into a full candidate or nightly.

## Publication boundary

After the gate merges the candidate PR, finalization creates or refreshes a
draft browser GitHub release targeted at the original candidate SHA and renders
a browser appcast preview. It never publishes the appcast.

Inspect the draft, retained lane evidence, and preview before promoting the
browser through the explicit release commands. Server OTA and extension feed
publication belong to their standalone workflows:

| Workflow | Owned publication |
| --- | --- |
| `release-server.yml` | BrowserOS server release, versioned/`latest` resources, and live alpha OTA by default |
| `release-claw-server.yml` | BrowserOS neo server release, versioned/`latest` resources, and live alpha OTA by default |
| `release-claw-onboard.yml` | onboarding release and resources |
| `release-extensions.yml` | extension CRX release, alpha feed snapshots, and CDN objects |
| `release-extension-feeds.yml` | extension manifest preview or publication |

A direct server dispatch defaults `publish_ota=true`. After the immutable
release and every `latest` alias are finalized, native jobs produce all five
platform appcast fragments, publish the live alpha appcast through the guarded
feed publisher, and commit the deployed snapshot under `updates/server/`. Use
`publish_ota=false` to stop after component finalization. Production promotion
remains an explicit `browseros ota server promote --product <id> --publish`.

The full browser workflows never invoke these standalone publishers. They only
consume resources built from their candidate and leave every component OTA
surface unchanged.

## Required configuration

The common producer needs Chrome, Bun, network access to the bundled extension
manifest, and the selected product's extension signing/build secrets. BrowserOS
native lanes need `BROWSEROS_CONFIG_URL`, `POSTHOG_API_KEY`, and `SENTRY_DSN`.
BrowserOS neo native lanes need `CLAW_POSTHOG_KEY` and Rust/Cargo. Browser
uploads use the `R2_*` secrets.

Windows signing needs the eSigner secrets and `SPARKLE_PRIVATE_KEY`. macOS uses
repository variables `BROWSEROS_REPO_PATH` and `BROWSEROS_CHROMIUM_SRC` plus
the signing and notarization secrets on the persistent builder. Runner labels,
cache behavior, and queue recovery are documented in `warpbuild-ci.md`;
persistent macOS setup is in `nightly-macos-ci.md`.
