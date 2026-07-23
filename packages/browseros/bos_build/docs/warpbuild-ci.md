# WarpBuild Release CI

The release Linux and Windows browser lanes run Chromium builds on WarpBuild
Azure BYOC runners:

- `.github/workflows/release-linux.yml`
- `.github/workflows/release-windows.yml`
- `.github/workflows/build-browseros.yml`

The per-platform workflows resolve product matrices and delegate each browser
build to the reusable `build-browseros.yml` workflow. That reusable workflow
owns the WarpBuild checkout, cache, sync, build, upload-artifact, and
queue-watchdog assumptions documented here. The signed macOS release and
nightly lanes use the repo-scoped Mac Mini runner instead; see
`release-ci.md` and `nightly-macos-ci.md` for those paths.

## Runners

| Platform | Label | Image | Compute | Disk |
| --- | --- | --- | --- | --- |
| Linux x64 | `warp-custom-browseros-ubuntu-2204-x64-32x` | Ubuntu 22.04 | `Standard_D32alds_v7` | P40, 2048 GB |
| Windows x64 | `warp-custom-browseros-windows-2025-x64-32x` | Windows Server 2025 | `Standard_D32als_v7` | P30, 1024 GB |
| macOS arm64 | `warp-macos-26-arm64-12x` | macOS 26 | M4 Pro, 12 vCPU / 44 GB | 500 GB |

WarpBuild provisions the Linux and Windows runners in the BrowserOS Azure
subscription through the `browseros-ci-eastus` BYOC stack in East US. Both
configurations are on-demand, with no standby pool and static IPs disabled.
They are ephemeral:
the VM and build disk are created for a job and discarded afterward. Their
compute SKUs and disk capacities mirror the source Azure build VMs, but they
do not clone or retain those VMs' persistent disks.

There is no 32-core macOS tier; 12x is WarpBuild's largest Mac. The macOS
label is kept in the runner catalog for future reusable `build-browseros.yml`
callers, but the current signed macOS release and nightly workflows use the
self-hosted `browseros-builder` runner. The macOS image version must satisfy
the chromium pin's SDK requirement — check
`build/config/mac/mac_sdk.gni` (`mac_sdk_official_version`) in the pinned
tree when bumping `CHROMIUM_VERSION`; chromium 148 needs the macOS 26 SDK,
and the macOS 15 image (Xcode 16.4 / SDK 15.5) fails compiling
`skia_utils_mac.mm` (`kCGImageByteOrder32Host` only exists in SDK 26).
WarpBuild runners register as self-hosted, so GitHub's 6-hour hosted-job
cap does not apply — but `timeout-minutes` must be set explicitly (the
implicit default is 360). The 2048 GB Linux and 1024 GB Windows build disks
leave ample headroom for the ~60-75 GB checkout and ~25-40 GB out dir. The
workflow prints `df -h` after each build. The WarpBuild BYOC connection and
runner configurations are the source of truth for labels, sizes, and Azure
placement; public docs pages can lag the live configuration.

## One-time setup (WarpBuild)

The `warpbuildbot` GitHub app is installed org-wide on `browseros-ai`
(since 2026-06-11). Two more things must be true before any `warp-*` job
leaves `queued`:

1. **The org must allow self-hosted runners on public repos.** WarpBuild
   runners register as org-level self-hosted runners, and GitHub blocks
   those on public repositories by default
   (https://www.warpbuild.com/docs/ci/public-repos). BrowserOS is public,
   so an org admin must check: Organization Settings → Actions → Runner
   groups → Default → "Allow public repositories". Via API (needs
   `admin:org` scope):

   ```bash
   gh auth refresh -h github.com -s admin:org
   gh api orgs/browseros-ai/actions/runner-groups \
     --jq '.runner_groups[] | {id, name, allows_public_repositories}'
   gh api -X PATCH "orgs/browseros-ai/actions/runner-groups/<id>" \
     -F allows_public_repositories=true
   ```

   Before flipping the toggle, check what else lives in that group — it
   widens exposure for every runner in it:

   ```bash
   gh api "orgs/browseros-ai/actions/runner-groups/<id>/runners" \
     --jq '.runners[] | {name, status, labels: [.labels[].name]}'
   ```

   Expect only ephemeral `warp-*` runners (usually none while idle). The
   signed-nightly Mac (`browseros-builder`) is registered at the repo
   level, so this org-group toggle does not change its exposure. If the
   group ever holds other persistent org-level runners, give WarpBuild a
   dedicated runner group instead of widening Default.

   Done for `browseros-ai` on 2026-06-13 — pickup verified live (a
   queued job was claimed within ~60 s of dispatch).

2. **The Azure BYOC connection must be healthy**: sign in at
   https://app.warpbuild.com/ and confirm the BrowserOS Azure subscription,
   stack `browseros-ci-eastus` in East US, and both custom runner
   configurations listed above. Also check Azure quota and regional capacity
   for their VM SKUs when provisioning fails.

Smoke test after changing either:
`gh workflow run release-linux.yml -f products=browseros -f upload_to_r2=false`,
then watch the build job leave `queued` within ~5 minutes (`gh run watch`).
Only do this when you intentionally want to spend Azure compute time.

## Release lane flow

`release-linux.yml` and `release-windows.yml` build one matrix entry per
selected product (`browseros`, `browserclaw`, or `all`) and call
`.github/workflows/build-browseros.yml` with `profile=release-ci`. Linux always
builds unsigned artifacts. Windows follows the caller's `sign` input and can be
used for unsigned verification with `sign=false`. Both lanes pass
`upload_to_r2` through to the reusable workflow.

The reusable workflow performs the per-platform recipe:

1. `actions/checkout` + `astral-sh/setup-uv`.
2. Restore the pinned chromium checkout from cache (see below).
3. `browseros source ensure --step checkout` — ensures depot_tools and
   `src` at the tag from `packages/browseros/CHROMIUM_VERSION`. No-op when
   the cache is warm and the pin unchanged.
4. `uv run browseros build --modules clean ...` — the standard clean module
   resets the tree (it also deletes hook-managed toolchains like
   `third_party/llvm-build`, which the next step restores).
5. `browseros source ensure --step sync` — `gclient sync -D
   --no-history --shallow`, exactly what the git_setup module runs.
6. Save the cache (only when the restore missed, i.e. first run per pin).
7. `uv run browseros build --profile release-ci --product <product>
   --arch <arch> --chromium-src .../src`, with signing and R2 upload flags
   resolved from workflow inputs.
8. Upload release build artifacts to the Actions run with 14-day retention.

The `release-ci` profile is the release preset minus `clean`/`git_setup`
(steps 4-5 replace them). Why not run `git_setup` as-is: it does
`git fetch --tags`, which on the shallow CI clone would pull objects for all
~70k chromium tags; the script instead fetches exactly the pinned tag at depth
2. On Windows the `mini_installer` module builds the installer that the signing
step signs when `sign=true`.

## Caching strategy

Cache key: `chromium-src-<platform>-<arch>-v1-<CHROMIUM_VERSION>`. Contents:
the whole gclient root (depot_tools, `.gclient`, post-sync `src`) captured
immediately after `gclient sync`, before patches and before any `out/` dir
exists — pristine and deterministic. The pin changes rarely, so steady state
is one cold sync per chromium bump per platform.

- **Linux / macOS — WarpCache** (`WarpBuilds/cache@v1`): drop-in for
  `actions/cache` with no size cap (entries expire 7 days after last use).
  Restore-keys fall back to the previous pin's cache, then the script
  fast-forwards `src` with a single-tag fetch.
- **Windows — R2 tarball** (`browseros source cache`): WarpCache does not
  support Windows runners and `actions/cache` caps at 10 GB/repo. The tree
  is zstd-tarred (~25-30 GB) into `ci-cache/chromium/` in the existing R2
  bucket using the same `R2_*` secrets the build already needs for
  `download_resources`. R2 has zero egress fees. Missing credentials or a
  cache miss degrade to a cold checkout, never a failure.

Expected timings (32-core linux/windows, M4 Pro mac):

| Phase | Cold (first run / pin bump) | Warm |
| --- | --- | --- |
| Checkout + sync | 40-70 min | restore 3-10 min + sync 5-15 min |
| Compile + package | 2.5-6 h (per platform) | same — out/ is rebuilt per run |
| Total | ~4-7 h | ~3-6 h |

The compile dominates either way; the cache removes the checkout cost and
network flakiness. Toolchains deleted by `clean` (~2-4 GB) are re-fetched by
hooks each run — accepted, matches the maintainer's local flow.

Linux and Windows compute and managed-disk costs accrue in the BrowserOS Azure
subscription; WarpBuild's managed-runner list prices do not apply. Use Azure
Cost Management for the actual BYOC cost and the WarpBuild account for any
separate platform or cache charges. Current signed macOS release and nightly
builds run on the self-hosted Mac Mini.

Azure BYOC does not support WarpBuild snapshot runners. Keep checkout
acceleration on WarpCache for Linux and the R2 tarball for Windows; do not add
`snapshot.key` runner syntax or `WarpBuilds/snapshot-save` to these lanes.

### Future optimizations (not yet wired)

- **Compiler cache (sccache/ccache via `cc_wrapper`)** in a CI gn flags
  variant: release rebuilds often differ only slightly, so this is the lever
  that could cut warm builds to well under an hour.
- **Linux arm64** via `architecture: [x64, arm64]` in the CI config once
  the x64 lane is green (sysroot bootstrap already handled by the modules).

## Operating release lanes

```bash
# BrowserOS Linux release artifact build without R2 upload.
gh workflow run release-linux.yml -f products=browseros -f upload_to_r2=false

# BrowserClaw unsigned Windows verification without R2 upload.
gh workflow run release-windows.yml \
  -f products=browserclaw \
  -f sign=false \
  -f upload_to_r2=false

# Both products on Linux with R2 upload.
gh workflow run release-linux.yml -f products=all -f upload_to_r2=true
```

The first run per platform is the cache warm-up; expect cold timings. If a
pin bump lands, the next run is cold again for that version. To force a fresh
checkout, bump the `v1` in the cache key (workflow) — for Windows also delete
the old object under `ci-cache/chromium/` in R2.

## Troubleshooting: jobs stuck in `queued`

A job no runner ever picked up shows `runner_id: 0` and empty steps:

```bash
gh run view <run-id> --json jobs --jq '.jobs[] | {name, status}'
gh api repos/browseros-ai/BrowserOS/actions/jobs/<job-id> \
  --jq '{status, runner_id, runner_name, labels}'
```

Causes, in the order to check:

1. **Runner group blocks public repos** — see one-time setup above. This
   stalls all platforms at once.
2. **Label does not match a live runner configuration** — compare the job
   label with the two exact custom labels above in
   https://app.warpbuild.com/. An unsupported label queues forever;
   WarpBuild reports no error back to GitHub.
3. **Azure BYOC stack** — confirm the connection and stack are healthy,
   then check subscription quota and East US capacity for the configured
   VM SKU.
4. **WarpBuild incident** — check their dashboard.

Mechanics worth knowing:

- GitHub discards self-hosted jobs queued for more than 24h, and each release
  platform workflow has a concurrency group (`release-linux` or
  `release-windows`) with `cancel-in-progress: false`. A queued build can
  therefore pin the platform lane for a full day. The `queue-watchdog` job
  steps in at the 20-minute mark: it cancels the run when no build job is
  actually running (everything stuck in queue or already finished), and fails
  loudly without cancelling while any build is in progress. In that mixed
  case, cancel the run manually once the live builds finish — a still-queued
  job otherwise pins the group for up to 24h with no watcher left.
- Fixing the root cause does not revive already-queued jobs: WarpBuild
  provisions on the `workflow_job.queued` webhook, which has already
  fired. Cancel the stuck run and re-dispatch.
- A job that IS picked up but dies in "Set up job" within seconds with
  `Unable to resolve action <owner>/<name>@vN` has nothing to do with
  WarpBuild: the floating major tag does not exist upstream (e.g.
  astral-sh/setup-uv publishes v8.x.y releases but no `v8` tag). Pin an
  exact existing version.
