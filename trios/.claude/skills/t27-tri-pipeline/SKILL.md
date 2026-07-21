---
name: t27-tri-pipeline
description: T27 tri-pipeline for trios - build, e2e, seal, verify, promote using Rust rings and build.sh. No .sh scripts per L7 UNITY.
argument-hint: [build|e2e|seal|verify|promote] [variant prod|staging]
---

# T27 tri-pipeline Skill (trios adaptation)

Adapted from `/Users/playra/t27/.claude/skills/tri-pipeline.md`. Replaces t27's `tri gen`/`tri test` with trios's Rust rings and `build.sh`.

## Commands

### `/t27-tri-pipeline build`

Run the canonical build for the requested variant.

```bash
# prod
./build.sh

# staging
TRIOS_VARIANT=staging cargo run --bin clade-build
```

Gates:
- Exit code 0.
- Produces `trios_app`.
- Produces `.app` bundle with valid `Info.plist`.

### `/t27-tri-pipeline e2e`

Run the canonical e2e for the variant.

```bash
# prod
cargo run --bin clade-e2e

# staging
TRIOS_VARIANT=staging cargo run --bin clade-e2e
```

Gates:
- Server health OK.
- App running.
- Swift logic tests pass.
- No critical log errors.

### `/t27-tri-pipeline seal`

Run the full tri-cell seal on the requested variant. Equivalent to the existing `/clade-seal` skill but invoked through T27 pipeline.

```bash
TRIOS_VARIANT=staging cargo run --bin clade-build
# Launch canary app
TRIOS_VARIANT=staging cargo run --bin clade-e2e
# Screenshot + log scan (handled by clade-e2e and clade-seal)
```

Cells:
1. Build passes.
2. Health probe returns `"status":"ok"`.
3. Screenshot baseline >= 95% similarity (if baseline exists).
4. E2E smoke passes.
5. Log scan shows 0 critical errors.

### `/t27-tri-pipeline verify`

Run Agent V conformance checks:

1. L1 TRACEABILITY: issue link present.
2. L2 GENERATION: spec/instruction justifies change.
3. L3 PURITY: ASCII-only check.
4. L4 TESTABILITY: build + e2e pass.
5. L5 IDENTITY: sacred constants preserved.
6. L6 CEILING: SSOT files unchanged unless spec says so.
7. L7 UNITY: no new `.sh`/`.py` on critical path.

Use `t27-verifier` agent for the actual verdict.

### `/t27-tri-pipeline promote`

Promote Canary to Sovereign after seal and verify.

```bash
cargo run --bin clade-promote
```

Gates:
- Seal passed.
- Verifier verdict = CLEAN.
- Safety budget > 0.
- Boot probe passes after atomic swap.

## Output Format

```
## T27 tri-pipeline - {command} {variant}
Status: {PASS|FAIL|BLOCKED}
Gates:
- Build: {PASS|FAIL}
- E2E: {PASS|FAIL}
- Seal: {PASS|FAIL}
- Verify: {PASS|FAIL|N/A}
- Promote: {PASS|FAIL|N/A}
Artifacts:
- {path}: {description}
```

## Trinity Compliance

- L4 TESTABILITY: all gates must pass.
- L7 UNITY: pipeline uses Rust rings and `build.sh`; no new shell scripts.
