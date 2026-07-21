---
name: phi-loop
description: PHI LOOP execution - guides AI through 9 phases of ring-based development for trios. Clade-aware, Rust-first.
parameters:
  - name: ring
    type: string
    description: Ring number (e.g. "SR-02" or "RUST-01")
  - name: phase
    type: string
    description: Target phase (issue, spec, tdd, impl, seal, verify, land, learn)
  - name: context
    type: string
    description: Optional context about the work
  - name: road
    type: string
    description: Road A (fast) / Road B (balanced) / Road C (deep) / Road B-clade (Canary seal)
---

# PHI LOOP Skill (trios adaptation, Clade-aware v2)

The PHI LOOP is a 9-phase development methodology for trios rings and UI components.
Clade phases added for self-improving agent safety.

## Phases

1. **Issue** - Define problem or requirement (GitHub issue #N)
2. **Spec** - Write agent instruction or skill spec in `.claude/`
3. **TDD** - Define test criteria: `cargo run --bin clade-build` + e2e + UI anomaly checklist
4. **Code/Impl** - Implement in Swift or Rust according to spec
5. **Gen** - Not applicable for trios (Swift/Rust is canonical source)
6. **Seal** - Tri-cell seal (build + health + screenshot) via `/clade-seal`
7. **Verify** - Agent V verdict (e-value gate + safety budget + empirical gates) via `/clade-promote`
8. **Land** - Merge changes to `dev` branch with `Closes #N` OR promote Canary to Sovereign
9. **Learn** - Capture learnings and update `.trinity/experience.md`

## Roads

### Road A (fastest) - Direct fix
- For critical hotfixes only
- Must snapshot Sovereign BEFORE change via `cargo run --bin clade-rollback`
- Build + health check, no full seal

### Road B (balanced) - Fix + test + experience save
- Standard ring development
- Full PHI LOOP phases 1-9
- Seal with `cargo run --bin clade-build` + `cargo run --bin clade-e2e`

### Road B-clade (Canary) - Safe experimentation
- Agent experiments in `.worktrees/staging` (canary branch)
- Full tri-cell seal via `/clade-seal`
- Agent V verdict via `/clade-promote`
- Boot probe after promote
- On failure: auto-rollback via `cargo run --bin clade-rollback`

### Road C (deep) - Spec-first, full PHI LOOP
- For architecture changes, new rings, agent spawn
- Write spec first, review with user
- Full seal + verify + clade archive update

## trios-Specific Verification

- **Build**: `cargo run --bin clade-build` must produce `trios_app` without errors
- **E2E**: `cargo run --bin clade-e2e` must show server OK + app running
- **UI**: Screenshot must pass anomaly checklist (no duplicate headers, tabs visible, glassmorphism active)
- **A2A**: `curl -s http://127.0.0.1:9105/health` must return `{"status":"ok"}`
- **Clade**: SHA-256 checksum valid, boot probe passes, e-value >= 5

## Clade-Specific Gates

| Gate | Threshold | Tool |
|------|-----------|------|
| Build time | <= 120% median | clade-build |
| Binary size | <= 110% median | clade-build |
| Launch time | <= 5s | curl health |
| Screenshot | >= 95% similarity | screencapture + diff |
| Log errors | 0 in 2m | log show |
| A2A register | <= 10s | curl |
| SHA-256 | valid | shasum -a 256 |
| E-value | >= 5.0 | .trinity/state/clade.json |
| Safety budget | > 0 | .trinity/state/safety_budget.json |

## Output Format

On phase completion, include:
```
Phase complete: [phase name]
-> Phase [next phase number]: [next phase name]
```

This triggers automatic branch creation for next phase if needed.

## Trinity Compliance
- L1 TRACEABILITY: `Closes #N` on every land/promote
- L4 TESTABILITY: Build + e2e + seal before merge
- L7 UNITY: Rust rings only, no .sh scripts
