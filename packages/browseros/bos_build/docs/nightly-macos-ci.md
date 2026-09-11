# Nightly macOS CI

The two manual entry workflows complete and publish one product each:

| Workflow | Display name | Rolling prerelease |
| --- | --- | --- |
| `.github/workflows/nightly-macos-browseros.yml` | Nightly: BrowserOS (macOS arm64) | `nightly-browseros` |
| `.github/workflows/nightly-macos-browserclaw.yml` | Nightly: BrowserOS neo (macOS arm64) | `nightly-browserclaw` |

Dispatch either entry from `main`; neither has a schedule or release-shape inputs.

```bash
gh workflow run nightly-macos-browseros.yml --ref main
gh workflow run nightly-macos-browserclaw.yml --ref main
```

`.github/workflows/reusable-build-macos-nightly.yml` is named
**Reusable: Build macOS nightly** and accepts only `workflow_call`. It retains the
existing signed arm64 build, provisioning profiles, published resources, deferred
R2 upload, and unconditional Chromium-workspace and signing-keychain cleanup.

## Product transaction

Each entry freezes one source SHA, reserves a browser version and its own server
and extension versions, then prepares those two exact components. BrowserOS pins
`server`, `agent`, and the existing `app-onboard` version. BrowserOS neo pins
`claw-server-rust`, `browserclaw`, and the existing `claw-onboard` version.
Onboarding bundles continue to release independently.

```text
manual dispatch from main
  -> reserve one product transaction on a draft PR
  -> prepare and verify its server and extension
  -> build one signed macOS arm64 browser
  -> finalize its server and extension; assemble its server appcast
  -> acquire release-feed-snapshots
  -> compose current-main snapshots and render its extension pin
  -> reconcile the product gate and merge state
  -> conditionally create or verify its immutable DMG and receipt
  -> publish current committed feeds and reconcile its rolling prerelease
```

A sibling build or finalizer is never a dependency. Component callers pass
`state_owner: suite` and disable standalone feed publication, leaving the product
transaction as the owner of its tracked state.

The retry identity is `nightly-<product>-<source-sha>` and the reservation branch
is `bot/release-nightly-<product>-<12-source-chars>`. Different products therefore
have different records even when dispatched at the same source commit. The
transaction PR is created as a draft. Open, closed, and merged canonical suite
records continue to reserve their versions; closing a failed PR does not free a
version already used by immutable uploads.

| Identity | Meaning |
| --- | --- |
| Source SHA | Frozen `main` commit used for artifact provenance and component builds |
| Reservation SHA | Immutable source child containing the exact version overlay used by the browser build |
| State SHA | Product PR head combining the reservation and current-main state with approved snapshots |
| Merge SHA | Squash commit proving the product state became durable on `main` |

Builders always check out the Reservation SHA. The state and merge commits are
never build inputs. After branch deletion, `refs/pull/<PR_NUMBER>/head` keeps the
validated reservation reachable for recovery. Shared browser version and build
offset allocation remains monotonic across both products and saved family records.

## Shared feeds and publication recovery

One `release-feed-snapshots` job holds the lock from current-main resolution
through state merge and publication. It writes only its server alpha appcast and
the three shared extension snapshots. Rendering pins only the selected extension;
unpinned entries carry their newer live or committed versions through
`--baseline-root`. This preserves a sibling whose snapshot merge succeeded but
whose R2 upload failed. Standalone extension publishers use the same boundary.

Final state composition keeps the checkout HEAD at Source SHA, restores the
selected snapshots from current main, and passes that main SHA as
`--state-base-sha`. Backend reconciliation preserves unrelated and sibling state
while retaining the original reservation and selected component pins.

Publication first verifies that the transaction is merged and its Merge SHA is
an ancestor of current main. It conditionally creates the product's immutable
DMG and receipt before exposing any mutable feeds. Identical bytes and transaction
bindings are accepted on retry; conflicts fail without overwrite. The publisher
restores current committed feed files, so recovery cannot overwrite a later
sibling snapshot with files from the old transaction merge.

Use **Re-run failed jobs** on the original run for post-build or post-merge
failures. An already-merged publication skips rendering and state reconciliation,
reuses the original signed Actions artifact, and publishes current committed
feeds. A new full run cannot restart an already-merged transaction. Signed
artifacts are retained for 14 days; recovery needs those exact bytes.

Live feeds retain downgrade guards; nightlies never pass `--allow-downgrade`.
Rolling publication is source- and checksum-aware: a newer product release is a
superseded/no-op, an exact partial draft resumes, and conflicting source/version
or asset identities fail. A release record and its live tag must resolve to the
same source before cleanup or replacement is allowed.

## Cutover and saved family runs

The old `.github/workflows/nightly.yml` dispatch entry and
`nightly-macos-product.yml` helper are removed from current main. Historical runs
retain their saved workflow definitions, including that helper. Both new entries
retain the historical top-level `release-suite` concurrency group with
`cancel-in-progress: false` and `queue: max`. This conservatively serializes whole
nightly runs during cutover: an active or rerun family transaction cannot overlap
a product transaction, while each new graph can succeed independently.

Legacy records without a product field remain family records with identity
`nightly-<source-sha>`. Inspect or recover those with the legacy CLI by omitting
`--product`, or rerun failed jobs on the original family run. Their draft PRs,
closed records, and pre-PR reservation branches remain allocation history. No
automatic cancellation, closure, or migration of those records occurs.

Historical standalone extension-feed runs saved before this change do not have
the shared publication lock; finish those runs before starting new nightlies.
Rewriting `main` history remains unsupported because reservation provenance must
remain auditable.

## Mac runner and resources

The reusable worker requires `[self-hosted, macOS, ARM64, browseros-builder]` and
shares `macos-build` with full releases. Component allocation retains
`release-component-allocation`; both locks retain pending jobs with `queue: max`.

The profile keeps `resource_mode: published` and executes:

```bash
uv run browseros build --profile nightly-macos --product <browseros-or-browserclaw> \
  --arch arm64 --resource-mode published --chromium-src "$CHROMIUM_SRC"
```

Set `BROWSEROS_REPO_PATH` to the persistent BrowserOS checkout and
`BROWSEROS_CHROMIUM_SRC` to the warm CI-owned Chromium clone base. The runner must
run in the logged-in GUI user's session with Xcode, depot_tools, `uv`, `gh`,
signing/notarization credentials, and enough disk for disposable workspaces.
The Chromium helper repairs the CI base and creates a run-specific APFS workspace;
both workspace and signing-keychain cleanup run under `if: always()`.

The optional `PROD_MACOS_BROWSEROS_PASSKEY_PROFILE_B64` and
`PROD_MACOS_BROWSERCLAW_PASSKEY_PROFILE_B64` secrets remain product-specific.
Missing profiles leave normal signing available without platform passkeys;
configured invalid profiles fail before the long build.
