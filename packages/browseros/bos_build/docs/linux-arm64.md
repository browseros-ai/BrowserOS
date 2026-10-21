# Neo Linux arm64 releases

Neo can build Linux arm64 packages with the existing x64 WarpBuild runner.
`bos_build` already selects `target_cpu = "arm64"`, installs Chromium's arm64
sysroot, downloads the published arm64 Neo server, and packages an arm64
AppImage and Debian archive. This design connects those capabilities to the
release workflow and its exact artifact checks.

## Interface and ownership

The Neo release selection is `linux_arch=all|x64|arm64`, defaulting to `all` so
routine releases include both Linux architectures. Explicit `x64` or `arm64`
selections narrow the artifact contract. The standalone/reusable Linux workflow
calls this input `arch` and keeps its `x64` default for BrowserOS and source-mode
callers. CLI defaults remain compatible with historical release receipts.

The deep module is the existing release metadata validator in `release/common.py`.
Its interface accepts the selected platforms and architecture sets, then checks
product, version, source commit, workflow provenance, and the exact artifact set.
GitHub drafts, download promotion, and appcast generation cross the same seam;
callers do not independently interpret which Linux packages constitute a release.

| Selection | Required Linux artifact keys |
| --- | --- |
| `x64` | `x64_appimage`, `x64_deb` |
| `arm64` | `arm64_appimage`, `arm64_deb` |
| `all` | all four keys above |

Missing or extra keys fail validation. Selecting arm64 must never silently
attach x64 packages left by another run. Archive names and existing download
aliases remain architecture-specific; the x64 aliases retain their historical
names.

```text
Neo dispatch: platforms + linux_arch + publication
  -> preflight validates and freezes the selection/source
  -> release-linux expands product x architecture
       -> existing x64 build runner
       -> published server for the target architecture
       -> Chromium build -> verified AppImage + Debian pair
       -> versioned artifacts and Linux release.json
  -> draft finalizer validates the selected exact artifact set
  -> explicit promotion uses that same architecture selection
```

## Metadata is one serialized handoff

Both architectures own entries in the same versioned `linux/release.json`.
Its existing read/merge/write operation is not atomic. The Linux matrix therefore
runs with `max-parallel: 1`, inside the existing `release-linux` concurrency group.
This deliberately serializes compilation as well as publication: it is the
smallest ownership rule that prevents lost updates without a new storage protocol
or another finalization job. Each architecture still has its own checkout cache
key and product output directory.

A retry of a failed architecture may use a later workflow attempt than an already
successful sibling. Linux metadata merges only when the release identity, source,
workflow run, and complete pinned resource versions match. Unpinned standalone
receipts are replaced across attempts because mutable `latest` resources may
have changed. Each Linux artifact records its
own workflow attempt. The platform-level attempt is present only when every
artifact belongs to that attempt; mixed attempts cannot claim a single attempt.
An explicitly requested attempt still fails closed for a mixed receipt. Other
platforms retain their existing merge rules.

## Build host and resource mode

Use the existing x64 Linux runner to cross-compile. AppImage tooling is selected
for the host, while its `ARCH` setting and Debian metadata describe the target.
Installing an arm64 appimagetool on the x64 host would break packaging.

This release path uses published resources: the Neo server release already
produces `linux-arm64`, and extensions/onboarding are architecture-independent.
The separate source-resource provider currently supports Linux x64 only. The
Linux workflow rejects source mode with an arm64-containing selection during
planning, before allocating an expensive browser runner. Adding source mode
requires a Cargo GNU cross-linker and its own server/lane contract work.

## Run an arm64 build

After this change is merged, dispatch from the default branch:

```sh
gh workflow run release-browserclaw.yml --ref main \
  -f platforms=linux -f linux_arch=arm64 -f publication=draft-only
```

This binds the browser to exact published-resource versions created by the same
release transaction and leaves a browser draft. `draft-only` creates private
resource drafts/versioned objects and does not promote their aliases or feeds.
Omit `linux_arch` to build both architectures. Releases selecting only macOS or
Windows accept the default without building Linux.

Expected arm64 browser assets:

```text
BrowserOS_neo_v<VERSION>_arm64.AppImage
BrowserOS_neo_v<VERSION>_arm64.deb
```

For standalone Linux builds using the existing published resources:

```sh
gh workflow run release-linux.yml --ref main \
  -f products=browserclaw -f arch=arm64 -f upload_to_r2=true
```

A standalone `arch=all` build requires one product and explicit `server_version`,
`extension_version`, and `onboarding_version` inputs. Otherwise two builds could
resolve different mutable `latest` resources hours apart. The full Neo workflow
supplies these exact versions automatically; an unpinned standalone request for
both architectures fails before any browser runner is allocated.
Server/onboarding pins must be numeric `MAJOR.MINOR.PATCH` releases; extension
pins use the existing numeric Chrome version format. Mutable aliases such as
`latest` and `alpha` are rejected both during planning and retry retention.

Pass `--linux-arch arm64` (or `all`) to `browseros release github create`,
`browseros release publish`, and `browseros release appcast` when binding those
operations to an architecture-selected release.
An explicit CLI architecture always binds exact pairs, including `x64`, without
requiring additional provenance flags. Existing generic CLI calls with no release
contract keep their legacy behavior; when a contract is requested and the Linux
architecture is omitted, that contract defaults to x64.

## Evidence required before shipping

The implementation checks the release tooling without generating Chromium build
files. A successful cross-build is still required to establish that the pinned
Chromium source and BrowserOS patches compile for Linux arm64.

On a real Linux arm64 machine, inspect the ELF architecture and Debian metadata,
install the Debian archive, extract and launch the AppImage, and verify browser
startup, the sandbox, Neo extension startup, and the bundled Neo server. Verify
these on the intended distribution baseline, including its shared-library/glibc
requirements. Cross-host packaging deliberately skips target execution and library
checks, so those checks alone cannot establish runtime compatibility.
