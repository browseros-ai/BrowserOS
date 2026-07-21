---
name: clade-promote
 description: Promote Canary clade to Sovereign after Agent V verdict + e-value gate + safety budget. Rust-first.
 parameters:
   - name: clade_id
     type: string
     description: Clade to promote, e.g. clade-1.1.0
   - name: variant
     type: string
     description: staging (always staging -> prod)
---

 # Clade Promote Skill (PHI LOOP Phase 8)

 Promotes a validated Canary clade to Sovereign ONLY after all gates pass.
 The Sovereign binary is never edited directly.

 ## Promotion Pipeline

 ### Step 1: Agent V Verdict (Safety Gate)

 - Load `.trinity/state/safety_budget.json`
 - Check `halted == false`
 - Check `budget > 0`
 - Load `.trinity/state/clade.json` for current e-value
 - Gate: `e_value >= 5.0` required for promote
 - If any gate fails -> REJECT, do NOT proceed

 ### Step 2: Snapshot Sovereign

 ```
 cargo run --bin clade-rollback
 ```

 Actually: call `CladeGuard.snapshotCurrentBinary()` to snapshot the CURRENT Sovereign before overwrite.

 ### Step 3: In-Place Swap

 ```
 cp .worktrees/staging/trios_app trios_app
 cp .worktrees/staging/trios_app trios.app/Contents/MacOS/trios
 ```

 ### Step 4: Boot Probe

 ```
 ./trios_app &
 sleep 10
 curl -s http://127.0.0.1:9105/health | grep '"status":"ok"'
 ```

 - If health FAIL -> trigger rollback immediately via `CladeGuard.bootProbe()`

 ### Step 5: Update State

 - `.trinity/state/clade.json`: set `status = "sovereign"`, update `timestamp`
 - `.trinity/clades/archive.json`: add new clade entry with `fitness_score`
 - `.trinity/clades/fitness.csv`: append new row
 - Git tag: `git tag clade-{version}`
 - Git branch: `git branch archive/clade-{version}`

 ### Step 6: Update Safety Budget

 - Success: `budget += 0.2` (cap `max_budget`)
 - Update `total_trials += 1`
 - Reset `halted = false` if applicable

 ## Rejection Protocol

 If any gate or boot probe fails:
 1. Log rejection to `.trinity/event_log.jsonl`
 2. Do NOT update Sovereign binary
 3. Update safety budget: `budget -= 1.0`, `total_failures += 1`
 4. If `budget <= 0` -> set `halted = true`, stop all auto-improvement
 5. Suggest manual hotfix via Road A

 ## Output Format

 ```
 Phase complete: Land
 -> Phase 9: Learn (/experience-save)
 ```

 Or on rejection:
 ```
 Phase complete: Land - REJECTED (reason: {gate_name})
 -> Phase 0: Rollback (clade-guard)
 ```

 ## Trinity Compliance
 - L1 TRACEABILITY: Every promote tagged `clade-X.Y.Z`
 - L4 TESTABILITY: Boot probe before declaring success
 - L7 UNITY: Rust tools only
