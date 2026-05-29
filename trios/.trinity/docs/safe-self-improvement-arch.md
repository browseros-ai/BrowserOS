# Safe Self-Improvement Architecture v2 — Trios Clade System

**Date:** 2026-05-29
**Branch:** feat/zai-provider
**Status:** Architecture proposal — pending implementation

---

## 1. The Problem: Why Agents Break Themselves

Current `clade-improve` pipeline has **4 fatal gaps** that let an agent self-modify into a broken state:

| Gap | Current State | Impact |
|-----|--------------|--------|
| **Fake Tests** | `run_tests()` returns `vec![passed: true]` for all 5 test types | Any change passes validation |
| **No Differential Testing** | No comparison of Sovereign vs Canary outputs on identical inputs | Regressions go undetected |
| **No Shadow Mode** | `Decision::ShadowMode` exists but has zero implementation | Cannot soak-test before promotion |
| **Staging ≠ Real Worktree** | `clade-build` for staging uses `.worktrees/staging/trios`, but there is no automated `git worktree` sync from Sovereign | Canary may be stale or manually edited, losing git lineage |

**Result:** Every time the agent tries to improve itself, it:
1. Makes changes in a poorly-isolated staging copy
2. Passes fake tests
3. Swaps binary into Sovereign without regression checks
4. Breaks, forcing manual rollback

---

## 2. Research Foundations

The v2 architecture is based on 2024–2025 research:

### [AgentDevel: Release Engineering for Self-Improving LLM Agents](https://www.arxiv.org/pdf/2601.04620)
- Treat self-improvement as **external release engineering**, not internal reflection
- Maintain a **single canonical version line** ("golden version")
- Use **flip-centered gating**: detect `pass→fail` regressions before promotion
- **Promote-or-discard**: RC is either promoted or thrown away; golden never regresses

### [Governed Capability Evolution](https://arxiv.org/html/2604.08059v5)
- Staged pipeline: validation → sandbox → **shadow deployment** → gated activation → monitoring
- **Rollback is first-class**: automatic restore on drift detection
- Four compatibility checks: interface, policy, behavioral, **recovery compatibility**
- Results: 67.4% task success with **zero unsafe activations**, 79.8% rollback success

### [ICAN-Deploy: Identity-Stable Canary Deployment](https://arxiv.org/html/2605.28097)
- Separate **capability names** (frozen, hashed into identity) from **capability versions** (mutable runtime)
- **Atomic promote** under lock + **rollback closure** on any exception path
- Verified by TLA+ model checking and 100 real canary cycles

### [MI9: Runtime Governance](https://arxiv.org/html/2508.03858v2)
- Real-time controls: telemetry, authorization, conformance, drift detection, graduated containment
- Catch **emergent behaviors** from dynamic planning

### [Self-Improving Architecture for Dynamic Safety](https://arxiv.org/abs/2511.07645v1)
- Dynamic policy synthesis: 234 new policies autonomously generated
- Attack Success Rate reduced from 100% → ~45%

---

## 3. Proposed v2 Architecture

### 3.1 Three Tiers (Strengthen Existing)

```
┌─────────────────────────────────────────┐
│  SOVEREIGN (Prod)  — Port 9105          │
│  Binary: trios_app                      │
│  Git: main branch (read-only to agent)  │
│  Auto-restart via launchd               │
└─────────────────────────────────────────┘
                    ↑ atomic swap (only after full seal)
┌─────────────────────────────────────────┐
│  CANARY (Staging) — Port 9205           │
│  Worktree: .worktrees/staging           │
│  Git branch: canary/clade-X.Y.Z         │
│  Sync: auto-pull from main every 15min  │
│  Lives in real git worktree             │
└─────────────────────────────────────────┘
                    ↑ dev proposes changes
┌─────────────────────────────────────────┐
│  DEV (Ephemeral) — Port 9305            │
│  Sandbox: /tmp/clade-dev/TICKET         │
│  Lifetime: single improvement ticket    │
│  Isolated: no network, capped resources │
└─────────────────────────────────────────┘
```

**Key rule:** Sovereign binary is **never modified in-place**. Only swapped atomically from Canary after Canary passes full seal.

### 3.2 Git Worktree Isolation (Fix RUST-10)

Current `clade-worktree` only does `git status`/`commit` inside an existing directory. It does not manage the actual `git worktree` infrastructure.

**Needed changes in `clade-worktree`:**

```rust
// New: ensure worktree exists and is synced from main
fn ensure_canary_worktree() {
    // 1. git worktree add .worktrees/staging canary/clade-X.Y.Z
    // 2. git fetch origin
    // 3. git reset --hard origin/main (or merge)
    // 4. Verify dirty == 0 before any build
}
```

**New behavior:**
- On agent wake: `clade-worktree status` checks if staging is behind `origin/main` > 3 commits → auto-sync
- Before any `clade-improve`: verify staging worktree exists and is clean
- After promotion: `clade-worktree sync` fast-forwards staging to new main

### 3.3 Differential Testing — The "Flip Analysis" (New RUST-11)

Inspired by AgentDevel's pass→fail detection. New binary: `clade-diff`

**Algorithm:**
1. Start both Sovereign (current binary) and Canary (candidate binary) on different ports
2. Send **identical input suite** to both:
   - Health probe: `GET /health`
   - Chat messages: 20 standard prompts
   - UI events: click sequences
   - A2A messages: agent heartbeat protocol
3. Compare **outputs bit-for-bit** (or within tolerance for non-deterministic parts like timestamps)
4. Calculate **flip score**: number of tests where Sovereign passed but Canary failed
5. **Reject if flip score > 0** — any regression is unacceptable

**Implementation sketch:**
```rust
struct DiffResult {
    test_id: String,
    sovereign_output: String,
    canary_output: String,
    exact_match: bool,
    tolerance_passed: bool,
    flip: bool, // sovereign_passed && !canary_passed
}

fn run_differential() -> Vec<DiffResult> {
    // Start sovereign on 9105 (existing)
    // Start canary on 9205 (from staging build)
    // Run test suite against both
    // Compare
}
```

**This replaces the fake `run_tests()` in `clade-improve`.**

### 3.4 Shadow Mode (New RUST-12)

Implement the `Decision::ShadowMode` that currently does nothing.

New binary: `clade-shadow`

**Behavior:**
- Canary binary is **not promoted** to Sovereign
- Instead, a lightweight **traffic splitter** routes 10% of real requests to Canary
- Canary processes them but responses are **logged, not returned to user**
- After **soak window** (minimum 10 minutes, configurable):
  - Compare latencies: Canary must be within +20% of Sovereign
  - Compare error rates: Canary must have zero new error types
  - Compare outputs: semantic similarity > 0.95
- If all pass → auto-promote via `clade-promote`
- If any fail → discard Canary, deduct safety budget

**This gives real-world validation before user-visible deployment.**

### 3.5 Safety Budget v2 (Update RUST-08)

Current budget: 5.0 max, -1.0 on reject, +0.2 on success.

**Problems:**
- Recovery is too slow (needs 5 successes to recover from 1 failure)
- No time-decay healing
- Single budget for all operations

**v2 proposal:**
```rust
struct SafetyBudgetV2 {
    total_budget: f64,        // max 5.0
    build_budget: f64,        // max 2.0 — separate for compilation attempts
    e2e_budget: f64,          // max 2.0 — separate for runtime tests
    shadow_budget: f64,       // max 1.0 — separate for shadow deployments
    halted: bool,
    last_success_ts: u64,     // epoch seconds
}
```

**Rules:**
- Time-decay recovery: `+0.05/hour` if no incidents in last 24h
- Separate halting: if `build_budget <= 0`, agent can still do e2e/shadow, just not compile
- **Hard halt** (all budgets 0 or `halted=true`) → requires **manual unlock** with git commit hash verification
- **Soft halt** (one budget 0) → agent switches to lower-cost validation (e.g., only static analysis)

### 3.6 Boot Probe Automation (Update RUST-08)

Current `boot_probe()` does `sleep(10); check_health()` but **does not restart Sovereign**.

**Fix:**
```rust
fn boot_probe() -> bool {
    // 1. Signal existing Sovereign to shutdown gracefully
    kill_sovereign_gracefully();
    
    // 2. Swap binary (already done in atomic_swap)
    // 3. Start new Sovereign via launchd or direct spawn
    let child = Command::new("/Users/playra/BrowserOS-full/trios/trios_app")
        .env("TRIOS_VARIANT", "prod")
        .spawn();
    
    // 4. Wait up to 30s for health OK
    for i in 0..30 {
        if check_health(SOVEREIGN_HEALTH) { return true; }
        thread::sleep(Duration::from_secs(1));
    }
    
    // 5. If fail → rollback immediately
    emergency_rollback();
    false
}
```

**Critical:** If boot probe fails, rollback must be **automatic and immediate**. Currently user has to manually run `clade-rollback`.

### 3.7 Experience-Driven Halting (Update RUST-07)

Current `clade-experience` suggests child clades but does not **halt** dangerous patterns.

**Add to `clade-experience`:**
```rust
fn should_halt_improvements(clusters: &[Cluster]) -> Option<String> {
    for cluster in clusters {
        // If this tag has >2 failures where agent broke itself
        let self_break_count = cluster.episodes.iter()
            .filter(|e| !e.success && e.anti_pattern.contains("agent_broke_itself"))
            .count();
        
        if self_break_count >= 2 {
            return Some(format!(
                "HALT: cluster '{}' has {} self-break episodes. Manual review required.",
                cluster.tag, self_break_count
            ));
        }
    }
    None
}
```

**Integration:** `clade-improve` Phase 0 should call `clade-experience` first. If halt reason returned → abort before any sandbox creation.

### 3.8 Cron Spam Prevention (Fix Immediate)

Current `clade-monitor` does not verify file existence before spawning commands.

**Fix in `clade-monitor`:**
```rust
fn run_external_check(script_path: &str) {
    if !std::path::Path::new(script_path).exists() {
        log_event("missing_script", script_path);
        // Do NOT retry for 1 hour
        return;
    }
    // ... spawn
}
```

Also add exponential backoff for any check that fails:
- 1st fail: wait base interval
- 2nd consecutive fail: wait 2x
- 3rd: wait 4x
- Max: 8x interval

---

## 4. Implementation Priority

### P0 — Halt the Bleeding (This Week)
1. **Fix fake tests** in `clade-improve/src/pipeline.rs` — replace hardcoded `passed: true` with real test execution
2. **Add file-exists guard** to `clade-monitor` + exponential backoff
3. **Add differential test** — minimum 3 health + 3 chat prompts compared between Sovereign and Canary

### P1 — Isolation (Next Sprint)
4. **Real git worktree** for Canary — `clade-worktree ensure` creates/validates worktree
5. **Boot probe automation** — auto-restart Sovereign + auto-rollback on failure
6. **Safety budget v2** — separate budgets + time-decay recovery

### P2 — Advanced Safety (Following Sprint)
7. **Shadow mode** — `clade-shadow` with 10% traffic soak
8. **Experience-driven halt** — `clade-experience` feeds into `clade-improve` Phase 0
9. **Identity-stable hash** — cryptographic capability identity separate from version

---

## 5. Ring Mapping

| Component | Current Ring | v2 Change |
|-----------|-------------|-----------|
| Build | RUST-01 `clade-build` | Add differential test trigger post-build |
| E2E | RUST-02 `clade-e2e` | Use real variant binaries, not just health check |
| Rollback | RUST-03 `clade-rollback` | Add auto-rollback on boot probe failure |
| Improve | RUST-04 `clade-improve` | Replace fake tests; call `clade-diff`; check experience halt |
| Monitor | RUST-05 `clade-monitor` | Add file-exists guard + backoff; add differential check scheduling |
| Dashboard | RUST-06 `clade-dashboard` | Show diff status, shadow mode status, budget breakdown |
| Experience | RUST-07 `clade-experience` | Add `should_halt_improvements()`; expose to improve Phase 0 |
| Promote | RUST-08 `clade-promote` | Budget v2; auto-restart; soak window before swap |
| Launchd | RUST-09 `clade-launchd` | Add Sovereign auto-restart on binary change |
| Worktree | RUST-10 `clade-worktree` | Add `ensure` command; auto-sync from main |
| Diff | **NEW RUST-11** | `clade-diff` — differential testing binary |
| Shadow | **NEW RUST-12** | `clade-shadow` — traffic splitting + soak testing |

---

## 6. Success Metrics

| Metric | Current | Target v2 |
|--------|---------|-----------|
| Self-break rate | ~80% (user reports) | <10% |
| False positive tests | 100% (all pass) | <5% |
| Rollback success | Manual only | 100% automatic |
| Time to detect regression | Never | <30 seconds |
| Shadow soak before promotion | 0 minutes | ≥10 minutes |
| Safety budget halts | Rare | Immediate on pattern |

---

## 7. Immediate Action Items

1. **Stop all auto-improvement** until P0 complete — set `safety_budget.json` to `halted: true`
2. **Archive `cron.stderr.log`** — it contains 35KB of spam; add log rotation
3. **Create RUST-11 skeleton** — `cargo new --bin clade-diff` in `rings/RUST-11/`
4. **Patch `clade-improve/run_tests`** — at minimum, run `cargo test` in sandbox and check exit code

---

*φ² + 1/φ² = 3 | TRINITY*
