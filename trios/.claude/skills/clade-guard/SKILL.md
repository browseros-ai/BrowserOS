---
name: clade-guard
 description: Health monitor + snapshot ring + emergency rollback. Swift CladeGuard actor + Rust clade-rollback CLI.
 parameters:
   - name: action
     type: string
     description: quick|snapshot|rollback|health|audit
   - name: variant
     type: string
     description: prod or staging (default prod)
---

 # Clade Guard Skill

 Monitors Sovereign and Canary health every 10s. Triggers rollback on corruption.
 Snapshot ring with SHA-256 checksums and rotation (max 10).

 ## Commands

 ### Quick Health Check
 ```
 curl -s http://127.0.0.1:9105/health | grep '"status":"ok"'
 curl -s http://127.0.0.1:9205/health | grep '"status":"ok"'
 ```

 ### Snapshot Current Binary
 ```
 # Swift: CladeGuard.snapshotCurrentBinary()
 # Or manual: cp trios_app .trinity/snapshots/trios_app-$(date +%s)-$(cat .trinity/state/clade.json | jq -r .id)
 ```

 ### Emergency Rollback
 ```
 cargo run --bin clade-rollback
 ```

 Finds newest snapshot by mtime, verifies SHA-256, copies to:
 - `trios_app`
 - `trios.app/Contents/MacOS/trios`

 ### Audit
 ```
 ls -lt .trinity/snapshots/ | head -12
 cat .trinity/clades/fitness.csv
 cat .trinity/state/safety_budget.json
 ```

 ## Snapshot Ring Rules

 - Max 10 snapshots + 10 `.sha256` files
 - Oldest auto-deleted by `CladeGuard.pruneOldSnapshots()`
 - Every snapshot gets `.sha256` sidecar
 - Rollback skips snapshots with invalid checksum
 - If no valid snapshots -> manual intervention required

 ## Boot Probe

 After any rollback or promote:
 - Wait 10s
 - `curl http://127.0.0.1:9105/health`
 - If fail -> log `Boot probe FAILED`, notify user

 ## Trinity Compliance
 - L4 TESTABILITY: Health checks every 10s
 - L7 UNITY: Rust CLI `clade-rollback`, Swift `CladeGuard`
