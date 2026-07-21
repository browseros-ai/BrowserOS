# Wave Loop 001 — trios weak-spot hardening plan

## Sources
- Safety audit by t27-creator (RECURSION-001 follow-up).
- Literature scan: AgentBreeder (NeurIPS 2025), SAHOO (2026), Alignment Flywheel (2026), Safety is Non-Compositional (2026), HMARL-CBF (NeurIPS 2025).

## Key research takeaways
1. Recursive self-improvement needs drift indices and constraint preservation (SAHOO) — map to trios L1-L7 laws and safety budget.
2. Multi-agent safety benefits from red/blue team modes (AgentBreeder) — trios already has clade-audit; add red-team adversarial tests.
3. Governance-centric MAS decouples decision from safety enforcement (Alignment Flywheel) — trios needs a ClaimManager ring to enforce coordination-law.md.
4. Safety is non-compositional: safe parts can compose into unsafe wholes — trios must audit cross-ring interactions, not just individual rings.
5. Runtime safety guarantees via Control Barrier Functions — trios equivalent: fail-closed guards (RecursionGuard, CladeGuard) with invariants proved by tests.

## Decomposed plan (P0 -> P5)

### P0 — Critical crash/safety fixes (this wave)
- [ ] trios-mesh: add workspace lints and replace production `.unwrap()` / `.expect()` with `?`/proper error handling.
- [ ] trios-mesh: fix NaN-panic `partial_cmp` unwraps in `router.rs` and `routing.rs`.
- [ ] CladeGuard: make rollback atomic and fail-closed on missing checksum.
- [ ] TerminalTabView / QueenStatusViewModel: replace `/bin/zsh -c` shell execution with tokenized `Process` or strict allowlist.
- [ ] WindowManager / LLMClient / main.swift: replace `fatalError` and force unwraps in startup path with failable returns.

### P1 — Determinism / L1-L7 hygiene
- [ ] build.sh / clade-build: remove hardcoded `/Users/playra/...`, derive from script location / `TRIOS_ROOT`; ASCII-only output.
- [ ] trios-config: fail closed when `TRIOS_ROOT` unset and canonical path missing; never fall back to `/tmp`.
- [ ] All tests: stop mutating global `TRIOS_ROOT`/`PATH`; use injectable `TestCtx`.
- [ ] Enforce ASCII-only CI lint on `.md`, `.swift`, `.rs`, `.sh` under `BR-OUTPUT/`, `rings/`, `.claude/`, `.trinity/`.
- [ ] Agent registry.json sync check in CI.

### P2 — Governance automation
- [ ] Implement `ClaimManager` ring (or extend clade-monitor) to enforce claim/queue/heartbeat protocol from coordination-law.md.
- [ ] Add `claim.mutation` gate: canon-file changes require a matching active claim event.
- [ ] Add red-team adversarial tests for ChatLogic recursive-launch blocklist.

### P3 — Observability / safety budget
- [ ] Expose safety-budget metrics via clade-dashboard.
- [ ] Add drift-index logging for spec-to-code deltas (track lines changed without spec update).

### P4 — Cross-ring compositional safety
- [ ] Audit all cross-ring command invocations for unsafe composition (clade-promote killing `trios`, clade-build writing to `/tmp`, etc.).
- [ ] Document ring interaction invariants and add integration tests.

### P5 — Hardened runtime paths
- [ ] Move singleton lock/PID, build logs, e2e logs, sandbox dirs from `/tmp` to `~/.trios/state/` or project-relative `.trios/`.
- [ ] Add file permissions audit (lock file `0o600`, directories `0o700`).

## This iteration goal
Land P0 items that are self-contained and do not require UI testing:
1. trios-mesh lint + NaN unwrap fixes.
2. CladeGuard atomic rollback + fail-closed checksum.
3. A spec file for each change under `.trinity/specs/`.
4. Verifier verdict for each.
5. Experience save at end of wave.

## [FUTURE OPTIONS]
1) `mesh-guards` — finish all `trios-mesh` production `.unwrap()` / `.expect()` replacements and add property-based tests for NaN/empty routing tables.
2) `swift-shell-free` — remove every `/bin/zsh -c` call site from `BR-OUTPUT/` and replace with tokenized `Process` + strict allowlist.
3) `claim-manager-ring` — implement the coordination-law.md claim/queue enforcement as a Rust ring integrated with clade-monitor.
