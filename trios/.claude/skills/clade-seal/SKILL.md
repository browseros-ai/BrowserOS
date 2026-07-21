---
name: clade-seal
 description: Tri-cell seal pipeline - build, health probe, screenshot baseline for Canary before promote. Rust-first, no .sh.
 parameters:
   - name: variant
     type: string
     description: prod or staging (default staging)
   - name: ring
     type: string
     description: Ring being sealed, e.g. SR-02
---

 # Clade Seal Skill (PHI LOOP Phase 6-7)

 The tri-cell seal validates that a Canary clade is safe to promote to Sovereign.
 Each cell must pass; any failure blocks promote and triggers rollback.

 ## Cell 1: Build (Seal-1)

 ```
 TRIOS_VARIANT=staging cargo run --bin clade-build
 ```

 - Must exit 0
 - Must produce `.worktrees/staging/trios_app`
 - Must produce `.worktrees/staging/trios-staging.app/Contents/Info.plist` with TRIOS_VARIANT=staging

 ## Cell 2: Health Probe (Seal-2)

 ```
 /Users/playra/BrowserOS-full/trios/.worktrees/staging/trios_app &
 sleep 5
 curl -s http://127.0.0.1:9205/health | grep '"status":"ok"'
 ```

 - Must return JSON with `"status":"ok"` within 5 seconds
 - If fail -> kill Canary, reject promote

 ## Cell 3: Screenshot Baseline (Seal-3)

 ```
 screencapture -x /tmp/trios_baseline_staging.png
 ```

 - Compare against `.trinity/baselines/sovereign.png` using perceptual hash
 - Similarity >= 95% required (no glitched rendering, no black rectangles)
 - If fail -> reject promote, save screenshot to `.trinity/baselines/rejected/`

 ## E2E Smoke Test (Seal-4)

 ```
 TRIOS_VARIANT=staging cargo run --bin clade-e2e
 ```

 - Report must show server OK + app running + no critical errors

 ## Log Scan (Seal-5)

 ```
 log show --predicate 'process == "trios-staging"' --last 2m --style compact | grep -iE "crash|fatal|error" | wc -l
 ```

 - Count must be 0

 ## Seal Verdict

 On completion:
 ```
 Phase complete: Seal
 -> Phase 8: Land (clade-promote)
 ```

 If ANY cell fails:
 ```
 Phase complete: Seal - REJECTED
 -> Phase 0: Rollback (clade-guard emergencyRollback)
 ```

 ## Trinity Compliance
 - L4 TESTABILITY: All 5 cells pass before promote
 - L7 UNITY: Rust tools only (clade-build, clade-e2e)
