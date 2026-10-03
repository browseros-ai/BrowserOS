# Automatic release build preparation

Chromium upgrades must not require an operator to repair each release runner.
The selected BrowserOS checkout's `CHROMIUM_VERSION` is the source of truth.
Bring stale checkouts to that exact tag, finish cleanup, synchronize dependencies
and hooks, then build from a prepared workspace.

## Source preparation and macOS workspace ownership

One source-preparation implementation in `bos_build` owns reset, exact-tag
acquisition, checkout alignment, `.gclient` preservation, and dependency sync.
`git_setup` remains its local-build adapter; `source ensure` is its provisioning
adapter. Preserve the existing explicit clean commands and local no-clean
behavior. Source reset must account for gclient-managed repositories, which are
not necessarily Git submodules.

The macOS persistent base is infrastructure-owned cache state. Refresh it before
APFS copying and hold the existing checkout lock through preparation and copy.
Each product receives an independent disposable workspace; a universal build
prepares once and retains its workspace through both architectures. Product-
qualified workspace markers and state files must survive multiple requested
copies and support complete cleanup after partial failure.

All cleanup that can delete dependency toolchains must finish before the final
sync. Builds consuming prepared copies skip source cleanup and provisioning,
but retain output/checkpoint cleanup and pruning of stale resource families in
the separate persistent BrowserOS repository. Source cache publication remains
after successful sync and output cleanup, before product patches and resources.

Do not use a moving upstream branch, fetch every Chromium tag, overwrite custom
`.gclient` settings, or interpret matching HEAD as proof that sync succeeded.
Keep this focused; no general workspace-provider framework is needed.

## Windows host prerequisites

The build currently selects the locally installed Windows toolchain through
`DEPOT_TOOLS_WIN_TOOLCHAIN=0`. Git and gclient cannot install a missing Windows
SDK in that mode. A separate setup module must determine the SDK required by the
selected Chromium source, ensure the exact supported SDK is installed on the CI
host, and verify its required headers/libraries/tools before configuration.

The Chromium 155 release failed because its required SDK directory
`10.0.28000.0` was absent. Official installation instructions specify SDK package
`10.0.28000.2270`. Resolve this through supported Microsoft provisioning and
explicit installed-file verification; do not change Chromium's SDK pin to an
older installed version.

## Implementation ownership

| Piece | Owned code | Integration contract |
| --- | --- | --- |
| Source preparation | `steps/source/provision.py`, `steps/setup/git.py`, `steps/setup/clean.py`, `cli/source.py`, source/workspace helpers, macOS workspace script and its release/nightly callers; source preparation block in `build-browseros.yml` | Preserve planner/step names where practical; communicate any shared registration changes before editing |
| Windows SDK | New Windows SDK setup module; its Windows-only registrations/planner integration; Windows prerequisite workflow block if needed | Do not change source-preparation blocks, Chromium patches, GN files, or the Chromium pin |
| Integration and release | Combined main review, full Neo workflow dispatch, run/artifact verification, delegated repairs for additional failures | Merge both implementations before dispatch; a queued or component-only success does not prove full-release success |

The two implementation pieces have independent interfaces, so empty skeleton
modules are unnecessary. Each worker uses an isolated worktree, designs the
details, completes independent review, and merges its scoped PR. Preserve both
intentions when integrating current main; never force-push.

## Acceptance

Exercise preparation using real disposable checkouts: missing tag, old HEAD,
dirty source and dependency repositories, interrupted sync, preserved developer
changes and `.gclient` configuration, checkout locking, multi-product copies,
and universal ordering. Verify Windows setup on a real Windows runner; Python
syntax or mocked installation alone is not evidence of SDK readiness.

The owner's Chromium constraints apply: no GN generation or cleanup and no
Chromium unit/browser test targets. Any Chromium source change requires the
specified `autoninja -C out/Default_browseros_arm64 chrome` build gate.

Completion requires merged changes and a successful full BrowserOS Neo release
workflow, including server, extension, Linux, Windows, macOS universal, and
browser draft creation. Verify the expected browser artifacts and source
identity. Production promotion remains a separate request.
