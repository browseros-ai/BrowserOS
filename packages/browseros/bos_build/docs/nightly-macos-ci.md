# Nightly macOS CI

Two signed arm64 nightlies run on the persistent Mac builder:

| Workflow | Product | Schedule | Rolling prerelease |
| --- | --- | --- | --- |
| `.github/workflows/nightly-browseros.yml` | BrowserOS | `0 4 * * *` | `nightly-browseros` |
| `.github/workflows/nightly-browserclaw.yml` | BrowserOS neo | `30 6 * * *` | `nightly-browserclaw` |

Both use the `macos-build` concurrency group, so a nightly and a signed macOS
release never mutate the persistent checkout at the same time.

## Build contract

Nightlies use the same source-resource planner as full releases:

```bash
cd packages/browseros
uv run browseros build \
  --profile nightly-macos \
  --product <browseros-or-browserclaw> \
  --arch arm64 \
  --source-sha "$(git rev-parse HEAD)" \
  --chromium-src "$CHROMIUM_SRC"
```

The profile is deliberately small:

```yaml
preset: release
resource_mode: source
```

`bos_build` performs the complete resource preparation. It builds the selected
product extension, onboarding, and the active arm64 server from the checkout;
resolves the pinned bug reporter CRX; validates and stages those resources;
then compiles, signs, packages, and uploads the browser. There is no parallel
workflow-only staging script and no component download from R2.

| Product | Source-built resources |
| --- | --- |
| BrowserOS | `agent`, onboarding, Bun server `darwin-arm64` |
| BrowserOS neo | `browserclaw`, onboarding, Rust server `darwin-arm64` |

Common resources are prepared once and the server once for the concrete
architecture. The local directory includes both source and browser version:

```text
packages/browseros/resources/binaries/prepared_common/
  <product>/<source-sha>/<browser-version>/
```

This matters for a manual BrowserOS nightly: it may change only the browser
version files without committing them. `--source-sha` still binds all component
manifests and lockfiles to HEAD, and rejects any other tracked change.

Nightlies do not allocate or change server, extension, or onboarding versions.
They do not create component tags/releases, move component `latest` aliases,
or publish server/extension update feeds. The only rolling publication is the
signed browser nightly prerelease.

## Browser version policy

BrowserOS is the only nightly that mutates browser version files:

- Scheduled: `offset+build`, commit enabled, R2 upload enabled.
- Manual default: `offset+build`, commit disabled, R2 upload enabled.
- Manual hotfix shape: `offset+patch`.
- Manual no-bump: `none`.

When commit is enabled, the workflow commits only:

```text
packages/browseros/resources/BROWSEROS_VERSION
packages/browseros/bos_build/config/BROWSEROS_BUILD_OFFSET
```

It pushes a run-specific `bot/nightly-macos-version-*` branch without
overwriting an existing branch, opens a PR, and tries squash merge or
auto-merge. If policy or checks prevent the merge, the PR stays open. Resource
provenance remains bound to the build's HEAD and semantic browser version.

BrowserOS neo reads the current browser version and never changes version
files.

## Manual runs

Open Actions, choose the product workflow, and select the source branch in the
native branch picker.

BrowserOS inputs:

- `bump`: `offset-only`, `offset+build`, `offset+patch`, or `none`.
- `commit_version`: create the run-specific version PR.
- `upload_to_r2`: omit browser upload while still producing the Actions DMG.

BrowserOS neo has only `upload_to_r2`.

Equivalent dispatches:

```bash
gh workflow run nightly-browseros.yml \
  --ref main \
  -f bump=offset+build \
  -f commit_version=false \
  -f upload_to_r2=false

gh workflow run nightly-browserclaw.yml \
  --ref main \
  -f upload_to_r2=false
```

Every successful run uploads its DMG as a 14-day Actions artifact. It then
replaces the corresponding rolling GitHub prerelease with `--latest=false`.

## Persistent runner setup

The runner must be repository-scoped and carry these labels:

```yaml
runs-on: [self-hosted, macOS, ARM64, browseros-builder]
```

Run it in the logged-in GUI user's session. Codesign and `xcrun notarytool`
need that user's keychain; daemon or SSH-only sessions commonly fail with
`User interaction not allowed`.

The machine needs:

- A persistent BrowserOS checkout.
- A persistent Chromium `src` checkout at the repository pin.
- `uv`, `gh`, Bun 1.3.6 support, Rust/Cargo, depot_tools, Xcode tools, and Chrome.
- The macOS signing identity and notarization credentials.
- Enough disk for the Chromium checkout, outputs, and all DMGs.

Set these repository variables:

| Variable | Meaning |
| --- | --- |
| `BROWSEROS_REPO_PATH` | Absolute path to the persistent BrowserOS checkout |
| `BROWSEROS_CHROMIUM_SRC` | Absolute path to Chromium `src` |
| `BROWSEROS_NIGHTLY_REF` | Optional scheduled source branch; defaults to the repository default branch |

The workflows pass build, extension, server, R2, signing, and notarization
secrets explicitly from GitHub Actions. `packages/browseros/.env` may still
provide `SLACK_WEBHOOK_URL` for the fallback failure ping, but it is not the
resource-build contract.

## Host boundary

Nightlies intentionally build only macOS arm64. A Mac can also run a universal
source plan and build both Darwin server targets, but it cannot produce the
Linux or Windows browser lanes. Full releases use the separate native matrix
described in `release-ci.md`.

## Troubleshooting

`User interaction not allowed`: run the runner as the logged-in GUI user and
verify `MACOS_KEYCHAIN_PASSWORD` and the signing identity.

`bun`, `cargo`, `chrome`, `gclient`, or `autoninja` not found: fix the runner
service PATH and restart it. BrowserOS neo installs the stable Rust toolchain
and Darwin arm64 target, but `rustup` itself must be available.

Prepared-resource identity mismatch: remove only the reported
`prepared_common/<product>/<source-sha>/<browser-version>` directory and rerun.
Do not switch the nightly to published resources to bypass validation.

No BrowserOS version PR: check `commit_version`, the bump mode, branch-push
permission, and open `bot/nightly-macos-version-*` PRs.

Artifact-only build: set `upload_to_r2=false`. The Actions DMG and rolling
nightly prerelease are still produced.
