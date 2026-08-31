# Nightly macOS CI

One dispatch-only workflow releases the BrowserOS family together:

| Workflow | Products | Rolling prereleases |
| --- | --- | --- |
| `.github/workflows/nightly.yml` | BrowserOS and BrowserOS neo | `nightly-browseros`, `nightly-browserclaw` |

The workflow has no release-shape inputs. It freezes one `main` commit, reserves
one shared browser version, prepares exact server and extension resources, builds
both signed arm64 browsers, and publishes family state only after both builds
succeed. `.github/workflows/nightly-macos-product.yml` is an internal reusable
builder; do not dispatch it directly.

## Family transaction graph

```text
dispatch from main
  -> freeze one artifact source SHA
  -> reserve every family version on one draft transaction PR
  -> prepare four exact private component releases
  -> verify every component source SHA and version
  -> signed BrowserOS macOS build ──────┐
     signed BrowserOS neo macOS build ─┴─ both must succeed
  -> finalize exact server and extension releases
  -> assemble both server appcasts and render shared extension feeds once
  -> reconcile five tracked snapshots on the transaction PR
  -> validate the complete gate, mark the PR ready, and exact-head squash merge
  -> conditionally create or verify both immutable signed browser artifacts
  -> publish the committed feeds and reconcile both rolling prereleases
```

The prepare phase may create private GitHub drafts and immutable versioned R2
objects because the browsers need exact downloadable resources. It does not
publish component releases, update `latest`, publish live feeds, or write
tracked state. Those public effects wait until both failure-prone Mac builds
have completed. Suite and standalone server finalizers share their retained
per-server workflow concurrency groups. Inside that serialized boundary,
`latest` advances only when the existing alias is older or missing, verifies
identical version/source/checksum bindings, and leaves newer aliases unchanged.

The component workflows remain independently dispatchable. Suite callers pass
`state_owner: suite`, which suppresses the component-owned reflection and feed
PRs without changing standalone defaults.

## Transaction identity and Git state

The retry identity is `nightly-<source-sha>`. It does not contain the workflow
run ID or `GITHUB_RUN_ATTEMPT`, so rerunning the same frozen source recovers the
same browser version, build offset, component pins, branch, and pull request.
The deterministic branch is `bot/release-nightly-<12-source-chars>`.

Four Git identities must not be conflated:

| Identity | Meaning | Used for |
| --- | --- | --- |
| Source SHA | Frozen `main` commit selected by the dispatch | Artifact provenance and every component build |
| Reservation SHA | One child of source containing only the deterministic browser/component version overlay | Both signed browser checkouts |
| State SHA | Exact transaction PR head: reservation plus reconciled snapshots | Snapshot checksums, gate identity, and exact-head merge validation |
| Merge SHA | Squash commit that makes the tracked state visible on `main` | Publishing the committed feed files |

Neither the mutable state SHA nor the merge SHA is a browser build source.
Other commits can reach `main` between dispatch and merge, so the squash tree
may include unrelated changes, and state reconciliation intentionally adds
tracked feed/appcast bytes that the browser does not need. Builders fetch the
deterministic transaction branch only to make the reservation reachable,
verify the reservation is its ancestor, and check out the exact reservation
SHA. After GitHub deletes that branch, retries fetch
`refs/pull/<PR_NUMBER>/head`; GitHub retains this PR-head ref after merge in
this repository.

Suite inspection proves that the reservation has exactly source as its parent,
contains only the expected version paths with record-derived content, and that
the live state head descends from it with changes limited to the five suite
state paths. This proof runs before workflow outputs are exposed to builders.

The transaction PR is created as a draft. The suite will not recover an
unexpected ready PR during initial reconciliation. Final recovery accepts a
ready PR only when all five snapshot checksums already match, and merge still
requires the matching complete gate and unchanged head. The ready transition
therefore occurs immediately before the exact-head squash helper.

## One tracked-state commit

The suite PR is the only Git writer for the orchestrated nightly. It reserves
the shared browser version and all in-repository component reflections, then
adds exactly these snapshots:

```text
updates/extensions/bundled-manifest.xml
updates/extensions/extensions.alpha.json
updates/extensions/update-manifest.alpha.xml
updates/server/appcast-claw-server.alpha.xml
updates/server/appcast-server.alpha.xml
```

Extension feeds are rendered once with both exact extension pins. Both server
appcasts arrive as workflow artifacts from the reusable OTA workflow. The suite
rejects missing snapshots, any other changed path, a changed PR head, or a gate
whose source, versions, state SHA, products, or checksums differ. `main` sees at
most one squash commit for the combined nightly state.

Suite PRs are also the durable browser-allocation ledger. Open, closed, and
merged canonical suite records all burn their browser version and build offset;
closing a failed transaction cannot make an already-uploaded version available
to another source. A closed record is allocation history only and cannot emit
build outputs or restart release execution. The same all-state PR ledger burns
the four releasable component pins after branch deletion. Closed/merged and
pre-PR branch records block collisions but never authorize standalone reuse.

The deterministic remote transaction branch is the allocation ledger before
the draft PR exists. Git push and PR creation are separate external writes, so
a runner may stop between them. Every browser, candidate, and standalone
component allocator scans those canonical refs, reconstructs and validates the
exact reservation commit, and treats its versions as burned. A retry reuses the
same branch and reservation; a later source allocates beyond it. Once the PR
exists, its marker and the branch are duplicate views of one allocation and
must agree.

Rewriting `main` history is unsupported. If a canonical reservation's frozen
source is no longer an ancestor of `main`, allocation fails closed instead of
ignoring that reservation: immutable effects may already use its versions.
Recovery requires an operator to audit those effects and explicitly remove the
invalid reservation; discovery never performs that cleanup automatically.

## Dispatch

Dispatch from `main`:

```bash
gh workflow run nightly.yml --ref main
```

The workflow requires the repository default branch, triggering ref, checkout,
and `github.sha` to identify the same `main` commit. Selecting another ref fails
before any allocation or publication. There is currently no cron trigger; an
external scheduler may dispatch this single family entrypoint when desired.

## Published-resource browser builds

Both browser jobs use the same profile and shared browser version:

```bash
cd packages/browseros
uv run browseros build \
  --profile nightly-macos \
  --product <browseros-or-browserclaw> \
  --arch arm64 \
  --resource-mode published \
  --chromium-src "$CHROMIUM_SRC"
```

The profile sets `preset: release` and `resource_mode: published`. Each job gets
the exact product server, product extension, and onboarding pins from the suite
record. The checkout is the immutable reservation overlay, while
`BROWSEROS_BUILD_SOURCE_SHA` remains the frozen source SHA. Reconciled tracked
state is never a browser input. The build does not use mutable `latest` or live
feed resolution to choose component versions.

Each successful product build uploads one signed DMG and its checksum-bearing
release receipt as a 14-day Actions artifact; signing runners do not write R2.
After both builds and the state merge, the final publisher conditionally creates
the versioned DMG and receipt in R2 before exposing any mutable feeds. Existing
identical bytes and transaction bindings are success, while any conflict fails
without overwrite. The same job then publishes committed feeds and reconciles
rolling prereleases.

A new whole-workflow run fails closed once the suite is merged. Recover a
post-build or post-merge publication failure with GitHub's **Re-run failed
jobs** action on the original run. That preserves successful build jobs and
reuses their exact Actions artifacts; do not use **Re-run all jobs** as an
artifact retry mechanism.

## Resumable and non-regressing publication

GitHub and R2 effects form a resumable saga, not an atomic transaction:

- An existing effect with the same source identity and checksums is success.
- A conflicting source binding or checksum is fatal.
- Versioned browser DMGs and `release.json` use conditional creation and exact
  byte verification; they are never overwritten by a retry.
- Live feed publication uses the default downgrade guard; the suite never
  passes `--allow-downgrade`.
- A rolling tag already carrying a newer embedded browser version is
  superseded/no-op. The workflow never deletes it.
- The same browser version on a different source is fatal.
- Replacing an older rolling tag additionally requires its target to be an
  ancestor of the frozen source SHA.
- A legacy rolling release whose browser version cannot be parsed fails closed.
- A release record and its live tag must resolve to the same source. If release
  deletion left only a tag, that tag gets the same ancestry classification
  before the reconciler may remove it.
- Draft creation, DMG upload, and publication are resumable writes. An exact
  partial draft resumes, and success requires a fresh read of the published
  tag, release identity, and asset digest. A published release missing its tag
  is verified and safely recreated instead of being mistaken for success.

The release notes embed the browser version and transaction source so future
retries can make this decision without relying on workflow-attempt identity.
If publication alone fails after merge, rerun the failed jobs on the original
run. That recovery revalidates the merged suite through its durable PR-head ref
without rerunning successful signing jobs; a new full run fails before builds.

## Browser version policy

The suite reserves `offset+build` once for the whole family by updating:

```text
packages/browseros/resources/BROWSEROS_VERSION
packages/browseros/bos_build/config/BROWSEROS_BUILD_OFFSET
```

Both products consume that exact shared version. Allocation considers the
committed version plus every canonical suite PR record. A downstream failure
leaves the number burned; a different source advances rather than reusing it.

## Mac runner boundary

Only the two browser jobs require:

```yaml
runs-on: [self-hosted, macOS, ARM64, browseros-builder]
```

They share the `macos-build` concurrency group with full releases. `queue: max`
retains pending jobs rather than replacing one when a newer run arrives. The
family workflow also uses a retained `release-suite` queue because cancellation
can strand valid saga effects.

Run the Mac runner in the logged-in GUI user's session. Codesign and
`xcrun notarytool` need that user's keychain. The machine needs:

- A persistent BrowserOS checkout.
- A persistent Chromium `src` checkout at the repository pin, dedicated to CI
  as the APFS clone base.
- `uv`, `gh`, depot_tools, Xcode tools, and Chrome.
- The macOS signing identity and notarization credentials.
- The `PROD_MACOS_BROWSEROS_PASSKEY_PROFILE_B64` repository secret containing
  BrowserOS's base64-encoded Developer ID provisioning profile. The signing
  helper decodes it into runner-owned temporary storage; BrowserOS validates
  and embeds it, and unconditional cleanup removes the temporary copy.
- The `PROD_MACOS_BROWSERCLAW_PASSKEY_PROFILE_B64` repository secret containing
  BrowserOS neo's profile for `com.browseros.BrowserClaw`. Apple profiles are
  App-ID-specific, so the BrowserOS profile cannot be reused for neo even though
  both apps use the same signing team and certificate.
- Both profile secrets are optional while Apple approval is pending. A missing
  profile leaves the corresponding app normally signed and usable but without
  macOS platform passkeys. A configured but invalid profile fails before the
  long build so releases cannot silently ship the wrong App ID authorization.
- Enough disk for two Chromium outputs and DMGs.

Set these repository variables:

| Variable | Meaning |
| --- | --- |
| `BROWSEROS_REPO_PATH` | Absolute path to the persistent BrowserOS checkout |
| `BROWSEROS_CHROMIUM_SRC` | Absolute path to the warm, CI-owned Chromium clone-base `src` |

Before every build, `.github/scripts/macos-chromium-workspace.sh` repairs the
CI-owned base and creates a run/attempt-specific APFS copy-on-write workspace.
The build, patches, outputs, and packaging remain inside that disposable copy.
Chromium workspace and signing-keychain cleanup both run under `if: always()`.
Never point `BROWSEROS_CHROMIUM_SRC` at a developer checkout.

## Troubleshooting

No transaction PR: inspect `Freeze and reserve family transaction`. The run
must be a dispatch from `main` and needs `contents: write` plus
`pull-requests: write`.

Transaction PR is draft: that is expected until both Mac builds, all four
finalizers, both appcast assemblies, shared feed rendering, and the complete
gate succeed. Do not mark it ready manually.

Browser job queued with no steps: bring an online runner with all four labels
into the repository. No public component finalization has happened yet.

PR branch is gone on a retry: merged retries intentionally use
`refs/pull/<PR_NUMBER>/head`. A failure means that ref did not resolve to the
recorded state SHA; do not substitute the merge SHA.

Rolling prerelease reports superseded: a newer browser version is already live,
so the stale transaction completed without moving the tag backward.

Feed publication refuses a downgrade: the live feed is newer than the
transaction. Leave it unchanged and inspect whether this is an intentionally
superseded retry.

`User interaction not allowed`: run the runner as the logged-in GUI user and
verify `MACOS_KEYCHAIN_PASSWORD` and the signing identity.

APFS setup fails: confirm the base is on APFS, is checked out at
`packages/browseros/CHROMIUM_VERSION`, and can create the adjacent owned
workspace directory. Ordinary local changes are repaired automatically.
