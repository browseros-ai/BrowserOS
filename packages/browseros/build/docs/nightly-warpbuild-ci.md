# Nightly WarpBuild Release Builds

`.github/workflows/nightly-release.yml` builds UNSIGNED release artifacts for
Linux x64, Windows x64, and macOS arm64 every night on WarpBuild cloud
runners, uploads them to the Actions run, and refreshes a rolling `nightly`
prerelease on GitHub. It complements (does not replace) the signed
self-hosted macOS nightly in `nightly-macos-build.yml`; once signing is wired
up here, that workflow can be retired.

## Runners

| Platform | Label | Specs | Disk |
| --- | --- | --- | --- |
| Linux x64 | `warp-ubuntu-2204-x64-32x` | 32 vCPU / 128 GB | 150 GB |
| Windows x64 | `warp-windows-2025-x64-32x` | 32 vCPU / 128 GB | 256 GB |
| macOS arm64 | `warp-macos-15-arm64-12x` | M4 Pro, 12 vCPU / 44 GB | 270 GB |

There is no 32-core macOS tier; 12x is WarpBuild's largest Mac. WarpBuild
runners register as self-hosted, so GitHub's 6-hour hosted-job cap does not
apply — but `timeout-minutes` must be set explicitly (the implicit default is
360). Linux's 150 GB disk is the tightest fit: ~60-75 GB checkout +
~25-40 GB out dir + OS image. The workflow prints `df -h` after each build.

## Per-night pipeline (per platform)

1. `actions/checkout` + `astral-sh/setup-uv`.
2. Restore the pinned chromium checkout from cache (see below).
3. `scripts/ci/setup_chromium.py --step checkout` — ensures depot_tools and
   `src` at the tag from `packages/browseros/CHROMIUM_VERSION`. No-op when
   the cache is warm and the pin unchanged.
4. `uv run browseros build --modules clean ...` — the standard clean module
   resets the tree (it also deletes hook-managed toolchains like
   `third_party/llvm-build`, which the next step restores).
5. `scripts/ci/setup_chromium.py --step sync` — `gclient sync -D
   --no-history --shallow`, exactly what the git_setup module runs.
6. Save the cache (only when the restore missed, i.e. first run per pin).
7. `uv run browseros build --config build/config/release.<os>.ci.yaml
   --chromium-src .../src`.
8. Upload artifacts (14-day retention); a follow-up job recreates the
   rolling `nightly` prerelease for scheduled main runs.

The `release.*.ci.yaml` configs are the release configs minus `clean`/
`git_setup` (steps 4-5 replace them), minus `sign_*`/`upload`. Why not run
`git_setup` as-is: it does `git fetch --tags`, which on the shallow CI clone
would pull objects for all ~70k chromium tags; the script instead fetches
exactly the pinned tag at depth 2. On Windows the new `mini_installer`
module builds the installer that `sign_windows` would otherwise build.

## Caching strategy

Cache key: `chromium-src-<platform>-<arch>-v1-<CHROMIUM_VERSION>`. Contents:
the whole gclient root (depot_tools, `.gclient`, post-sync `src`) captured
immediately after `gclient sync`, before patches and before any `out/` dir
exists — pristine and deterministic. The pin changes rarely, so steady state
is one cold sync per chromium bump per platform.

- **Linux / macOS — WarpCache** (`WarpBuilds/cache@v1`): drop-in for
  `actions/cache` with no size cap (entries expire 7 days after last use;
  storage $0.20/GB-month). Restore-keys fall back to the previous pin's
  cache, then the script fast-forwards `src` with a single-tag fetch.
- **Windows — R2 tarball** (`scripts/ci/r2_cache.py`): WarpCache does not
  support Windows runners and `actions/cache` caps at 10 GB/repo. The tree
  is zstd-tarred (~25-30 GB) into `ci-cache/chromium/` in the existing R2
  bucket using the same `R2_*` secrets the build already needs for
  `download_resources`. R2 has zero egress fees. Missing credentials or a
  cache miss degrade to a cold checkout, never a failure.

Expected timings (32-core linux/windows, M4 Pro mac):

| Phase | Cold (first run / pin bump) | Warm |
| --- | --- | --- |
| Checkout + sync | 40-70 min | restore 3-10 min + sync 5-15 min |
| Compile + package | 2.5-6 h (per platform) | same — out/ is rebuilt nightly |
| Total | ~4-7 h | ~3-6 h |

The compile dominates either way; the cache removes the checkout cost and
network flakiness. Toolchains deleted by `clean` (~2-4 GB) are re-fetched by
hooks each night — accepted, matches the maintainer's local flow.

Cost ballpark at WarpBuild list prices: linux 32x $3.84/h, windows 32x
$7.68/h, mac 12x $9.60/h → roughly $60-120 per full nightly, plus ~$12/month
cache storage.

### Future optimizations (not yet wired)

- **Snapshot runners (Linux only)**: boot from a disk image with the
  checkout baked in (`runs-on: "warp-ubuntu-2204-x64-32x;snapshot.key=..."`
  + `WarpBuilds/snapshot-save`). Kills even the cache-restore minutes, but
  snapshots expire after 15 days and need a bake/boot split; revisit once
  the cache flow is proven.
- **Compiler cache (sccache/ccache via `cc_wrapper`)** in a CI gn flags
  variant: nightly sources are nearly identical night-to-night, so this is
  the lever that could cut warm builds to well under an hour.
- **Linux arm64** via `architecture: [x64, arm64]` in the CI config once
  the x64 lane is green (sysroot bootstrap already handled by the modules).

## Signing later (placeholders)

The workflow leaves named-but-unused secret placeholders documented next to
the build step. To enable signing:

- **macOS**: add `sign_macos` (and optionally `sparkle_sign`) back to
  `release.macos.arm64.ci.yaml`, provide `MACOS_CERTIFICATE_P12` +
  `MACOS_CERTIFICATE_PWD` (import into a temporary CI keychain in a step
  before the build), `MACOS_CERTIFICATE_NAME`, `MACOS_KEYCHAIN_PASSWORD`,
  `PROD_MACOS_NOTARIZATION_APPLE_ID/_TEAM_ID/_PWD`, `SPARKLE_PRIVATE_KEY`.
- **Windows**: replace `mini_installer` with `sign_windows` in
  `release.windows.ci.yaml`, install SSL.com CodeSignTool in a step, set
  `CODE_SIGN_TOOL_PATH` and `ESIGNER_USERNAME/_PASSWORD/_TOTP_SECRET`
  (+ `ESIGNER_CREDENTIAL_ID`).
- **Linux**: unsigned by design (no sign module in the release config).

## Operating it

```bash
# Full run on all platforms (no prerelease update):
gh workflow run "Nightly Release Build"

# One platform while iterating:
gh workflow run "Nightly Release Build" -f platforms=linux

# Manual run that also refreshes the rolling prerelease (main only):
gh workflow run "Nightly Release Build" -f publish_nightly=true
```

The first run per platform is the cache warm-up; expect cold timings. If a
pin bump lands, the next night is cold again for that version. To force a
fresh checkout, bump the `v1` in the cache key (workflow) — for Windows also
delete the old object under `ci-cache/chromium/` in R2.
