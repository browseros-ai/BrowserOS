---
description: PHI LOOP execution — guides AI through 9 phases of ring-based development for trios
parameters:
  - name: ring
    type: string
    description: Ring number (e.g., "SR-02")
  - name: phase
    type: string
    description: Target phase (issue, spec, tdd, impl, seal, verify, land, learn)
  - name: context
    type: string
    description: Optional context about the work
---

# PHI LOOP Skill (trios adaptation)

The PHI LOOP is a 9-phase development methodology for trios rings and UI components.

## Phases

1. **Issue** - Define problem or requirement (GitHub issue #N)
2. **Spec** - Write agent instruction or skill spec in `.claude/`
3. **TDD** - Define test criteria: `./build.sh` + e2e + UI anomaly checklist
4. **Code/Impl** - Implement in Swift according to spec
5. **Gen** - Not applicable for trios (Swift is canonical source)
6. **Seal** - Verify build and run e2e; capture screenshot baseline
7. **Verify** - Run tests, check UI anomalies, review screenshot
8. **Land** - Merge changes to `dev` branch with `Closes #N`
9. **Learn** - Capture learnings and update `.trinity/experience.md`

## Usage

When this skill is invoked:

1. Determine current phase from branch name or task context
2. Execute the appropriate phase actions
3. Provide clear output when phase is complete
4. Suggest next phase with explicit "→ Phase {N}" notation

## trios-Specific Verification

- **Build**: `./build.sh` must produce `trios_app` without errors
- **E2E**: `bash e2e/trios_e2e_flow.sh` must show server OK + app running
- **UI**: Screenshot must pass anomaly checklist (no duplicate headers, tabs visible, glassmorphism active)
- **A2A**: `curl -s http://127.0.0.1:9105/health` must return `{"status":"ok"}`

## Output Format

On phase completion, include:
```
Phase complete: [phase name]
→ Phase [next phase number]: [next phase name]
```

This triggers automatic branch creation for next phase if needed.
