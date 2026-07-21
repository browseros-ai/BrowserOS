---
name: t27-phi-loop
description: T27 PHI LOOP for trios - 9-phase spec-first development for canon Swift/Rust files. No .sh scripts per L7 UNITY.
argument-hint: [ring] [phase] [context] [road A|B|B-clade|C]
---

# T27 PHI LOOP Skill (trios adaptation)

The PHI LOOP is a 9-phase development methodology for trios canon files (`BR-OUTPUT/`, `rings/`, Rust rings). Adapted from `/Users/playra/t27/.claude/skills/phi-loop.md`.

## Phases

1. **Issue** - Define problem or requirement (GitHub issue #N).
2. **Spec** - Write a `.trinity/specs/*.md` behavior specification.
3. **TDD** - Define test criteria: build, unit tests, e2e, UI anomaly checklist.
4. **Code/Impl** - Implement according to spec (Swift or Rust).
5. **Review** - t27-creator output reviewed by t27-verifier (trios has no `tri gen`; this phase replaces t27's Gen).
6. **Seal** - Run `/t27-tri-pipeline seal` (build + health + screenshot + e2e + log scan).
7. **Verify** - t27-verifier verdict (L1-L7) and safety budget check.
8. **Land** - Merge/promote with `Closes #N`.
9. **Learn** - `/t27-experience-save` records the episode.

## Roads

### Road A (fastest) - Direct fix
- For critical hotfixes only.
- Snapshot Sovereign BEFORE change via `cargo run --bin clade-rollback`.
- Build + health check, post-hoc seal.
- Requires t27-queen approval and Agent V waiver comment.

### Road B (balanced) - Fix + test + experience save
- Standard canon development.
- Full PHI LOOP phases 1-9.
- Seal with `cargo run --bin clade-build` + `cargo run --bin clade-e2e`.

### Road B-clade (Canary) - Safe experimentation
- Agent experiments in `.worktrees/staging` (canary branch).
- Full tri-cell seal via `/t27-tri-pipeline seal`.
- Agent V verdict via `/clade-promote` or `/t27-tri-pipeline promote`.
- Boot probe after promote.
- On failure: auto-rollback via `cargo run --bin clade-rollback`.

### Road C (deep) - Spec-first, full PHI LOOP
- For architecture changes, new rings, agent spawn.
- Write spec first, review with user.
- Full seal + verify + clade archive update.

## Usage

When invoked:

1. Determine current phase from branch name (`ring-NNN-PHASE`) or argument.
2. Acquire claim per `.trinity/policy/coordination-law.md`.
3. Execute phase actions.
4. Provide clear output on completion.
5. Suggest next phase with explicit marker:

```
Phase complete: [phase name]
-> Phase [next phase number]: [next phase name]
```

## Output Format

```
## T27 PHI LOOP - {Ring} {Phase}
Road: {A|B|B-clade|C}
Claim: {claim_id}
Status: {DONE|BLOCKED|TOXIC}
Artifacts:
- {path}: {description}
Next:
-> Phase {N}: {name}
```

## Trinity Compliance

- L1 TRACEABILITY: `Closes #N` on every land/promote.
- L2 GENERATION: Spec/instruction is SSOT; Swift file is derived artifact.
- L4 TESTABILITY: Build + e2e + seal before merge.
- L7 UNITY: Rust rings and `build.sh` only; no new `.sh` scripts.
