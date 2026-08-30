# Design — Family-wide release transaction

**Chosen approach:** Dedicated suite transaction with component ownership seams — it centralizes identity, allocation, state reconciliation, and merge while retaining tested component release machinery.

## Considered approaches

### A. Dedicated suite transaction with component ownership seams (selected)

Add a family-level record/backend/CLI beside the existing candidate and component modules. The suite allocates all product versions, creates one deterministic PR, exposes the frozen source and current state heads, reconciles the complete snapshot set, and exact-head merges only after a family gate. Existing component workflows receive one ownership context so their standalone default remains unchanged while the suite suppresses their independent state writers.

This keeps workflow YAML declarative: it expresses the saga ordering and passes the suite record's exact pins, but identity/recovery/conflict policy lives in Python. It also lets existing standalone and per-product candidate allocation discover open suite reservations.

- Advantages: deep API; one durable retry record; testable without Actions; compatible standalone workflows; directly enforces one PR.
- Disadvantages: introduces a second release-record schema beside per-product candidates; requires careful cross-allocation discovery.
- Complexity: High.
- Risk: Medium.

### B. Generalize the existing candidate model from one product to a product family

Change `CandidateRecord` and `GitHubCandidateBackend` so one candidate may contain multiple products, snapshots, and progressive state commits. Existing full-release candidate tests and consumers would need a schema migration or compatibility layer.

- Advantages: one conceptual record type for future full releases.
- Disadvantages: widens and destabilizes a currently product-scoped module; forces the first nightly slice to migrate unrelated full-candidate contracts.
- Complexity: High.
- Risk: High.

### C. YAML orchestration plus shell-owned transaction branch

Create a combined workflow that allocates versions and pushes a deterministic branch with shell steps, then call the existing component workflows behind skip/defer flags.

- Advantages: fewer Python files initially.
- Disadvantages: retry identity, PR validation, conflict detection, and state ownership remain scattered and difficult to behavior-test; repeats the architecture problem at a larger YAML scale.
- Complexity: Medium.
- Risk: High.

## Architecture

`bos_build.release.suite` owns the transaction aggregate. A `SuiteRecord` binds `mode + source_sha` to one branch, reservation commit, live state head, browser version, all four releasable component versions, the exact onboarding pin, and one PR. The GitHub backend creates the reservation commit in a temporary worktree, recovers an identical interrupted push, rejects canonical-branch or marker conflicts, and reconciles only the allowlisted tracked snapshot set.

The combined nightly workflow first reconciles the record, then asks component workflows to prepare exact private releases from `source_sha`. Both macOS jobs fetch the durable transaction/PR-head ref but check out the proven immutable `reservation_sha`, which is the version overlay whose only parent is `source_sha`; release metadata keeps `source_sha` as artifact provenance. The later `state_sha` is reserved for tracked snapshot reconciliation and is never a browser input. Only after both builds succeed does the workflow finalize the prepared components and assemble two server snapshots. The retained per-server workflow concurrency groups reconcile mutable `latest` aliases monotonically for suite and standalone callers. One final Ubuntu job renders extension feeds once with both pins, reconciles all five snapshot files to the transaction PR, validates the family gate, and exact-head squash-merges it.

The publication tail checks out the returned merge commit, conditionally creates or verifies both immutable signed browser objects, and only then publishes the tracked feed/appcast files and rolling aliases. Rolling nightly GitHub releases reconcile after both builds: identical source and asset digests are a no-op, same-source digest conflicts fail, the same browser version on another source fails, and a newer embedded browser version is superseded/no-op. An older rolling source may be replaced only when its target is an ancestor of the frozen source. Release records and live tags must resolve to the same source; a tag left behind by a failed delete is independently classified before cleanup, and an exact partial draft resumes its missing upload/publication steps. The reconciler re-reads the final tag, release, and asset digests before reporting success. Component, browser-artifact, and feed effects remain saga steps; pre-merge retries inspect/reuse them rather than allocate new identities, while post-build recovery reruns only failed jobs from the original workflow so successful signed artifacts are reused.

## Key decisions

- One family record, not two product candidates → the current candidate identity is explicitly product-scoped (`packages/browseros/bos_build/release/candidate.py:36-75`) and its branch includes the product (`candidate.py:174-176`).
- Reuse component allocation and stamping → component manifests, tag namespaces, normalization, and candidate allocation already share one registry (`packages/browseros/bos_build/release/components.py:21-128`, `components.py:226-260`).
- Keep suite reservation discoverable by standalone allocators → the canonical remote transaction branch is the durable ledger before draft-PR creation; browser, candidate, and standalone allocation all validate it, then deduplicate the later PR-marker view through one suite-owned conversion helper.
- One validated `state_owner` seam → server and extension workflows currently run independent reflection jobs after finalization (`.github/workflows/release-server.yml:473-535`, `.github/workflows/release-extensions.yml:772-850`); ownership, rather than several skip flags, is the stable distinction.
- Assemble snapshots before state merge, publish after it → server OTA currently persists and publishes in one job (`.github/workflows/publish-server-ota.yml:314-365`); suite ownership splits that seam by uploading the assembled snapshot for the family reconcile.
- Preserve exact-head squash protection → the existing helper checks `--match-head-commit` and fails on changed PR heads (`packages/browseros-agent/scripts/release/merge-release-pr.sh`).
- Keep the first workflow dispatch-only → current behavior-level tests explicitly require no schedule (`packages/browseros/bos_build/ci_workflow_test.py:1269-1296`).

## Risks / trade-offs

- Git push and draft-PR creation are separate external writes. The remote transaction branch burns every reserved version during that crash window; later PR-marker and branch views must agree. Tests interrupt immediately after push and cover browser, standalone, and legacy candidate discovery.
- Main history rewrites are unsupported. A canonical reservation whose source is no longer on main fails allocation closed because its versions may already identify immutable effects; recovery is an explicit, audited removal rather than automatic stale-branch filtering.
- `state_sha` changes when snapshots are reconciled. The immutable PR marker stores reservation identity, while inspection derives the current state head from GitHub, avoiding an impossible atomic update across Git and PR metadata.
- GitHub and R2 cannot commit atomically with main. The ordered saga can leave already-valid effects after a later failure, but stable identity and exact-content checks make the next attempt resume safely.
- The full-release workflows remain product-specific in this slice. Supporting `full` in the record schema avoids baking “nightly” into branch/retry identity, but full workflow migration is intentionally deferred.

## Rejected alternatives

- Generalize `CandidateRecord` now — too much compatibility risk for the first production vertical slice.
- Shell-only transaction state — does not provide the requested deep reconcile/inspect ownership boundary.
- Permanent `feat/release-version` branch — retries and concurrent sources would race.
- One PR per component followed by a wrapper squash — main would still have multiple writers and retry identities.
