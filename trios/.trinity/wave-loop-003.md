# Wave Loop 003 — trios shell-free terminal, portable build paths, and agent/skill ASCII purity

## Sources
- Wave 3 audit and literature search from `/loop 15m` macro.
- Prior wave plans: `wave-loop-001.md`, `wave-loop-002.md`.

## Key research takeaways
1. Execution isolation via strict allowlists (IsolateGPT, AgentBound) maps directly to `TerminalCommandSanitizer` rejecting shell metacharacters.
2. Static capability manifests and data-flow validation (Haven) justify hardcoding no absolute paths in build tooling.
3. Agent prompt injection work (IterInject, ChatInject) shows regex blocklists are bypassable; trios uses allowlists and tokenized `Process()`.
4. Tool life-cycle threat models (OpenClaw) show execution-stage defenses are the highest-leverage hardening surface in the UI.

## Decomposed plan (P0 -> P5)

### P0 — Critical shell-safety and portability
- [x] Replace `TerminalTabView.runCommand` shell invocation with tokenized `Process()` and strict command allowlist.
- [x] Remove hardcoded `/Users/playra/BrowserOS-full/trios` from `clade-build`.
- [x] Remove hardcoded paths and non-ASCII markers from `build.sh`.
- [x] Move clade-build logs from `/tmp` to `.trinity/logs/`.

### P1 — ASCII purity across policy, agents, and skills
- [x] ASCII-clean all `.claude/agents/*.md`.
- [x] ASCII-clean all `.claude/skills/*/*.md`.
- [x] Update `t27-wave-loop/SKILL.md` with the actual macro process.
- [x] Create `ascii-lint/SKILL.md` for reusable ASCII cleanup.

### P2 — Specs and verification
- [x] Write `terminal-shell-free.md` spec.
- [x] Write `build-cleanup.md` spec.
- [x] Run `./build.sh`, `cargo test --workspace`, `cargo clippy -p clade-build`.
- [x] Run ASCII scan over source/agents/skills.

### P3 — Experience and institutional memory
- [x] Append Wave 003 episode to `.trinity/experience.md`.
- [x] Write `.trinity/experience/2026-07-21_123000_WAVE-003.json`.

### P4/P5 — Backlog
- [ ] Unit tests for `TerminalCommandSanitizer`.
- [ ] CI gate that fails on non-ASCII characters in `.claude/agents`, `.claude/skills`, `BR-OUTPUT`, `rings`, and `.trinity/specs`.
- [ ] Promotion lock between `clade-promote` and `clade-monitor`.

## Verification
- `./build.sh`: PASS
- `cargo test --workspace`: PASS
- `cargo clippy -p clade-build --all-targets --all-features`: CLEAN
- ASCII scan (`grep -RIn '[^\x00-\x7F]'`) over `BR-OUTPUT/*.swift`, `build.sh`, `rings/RUST-01/clade-build/src/main.rs`, `.claude/agents`, `.claude/skills`: CLEAN

## [FUTURE OPTIONS]
1) `terminal-tests-and-ci` — add unit tests for `TerminalCommandSanitizer`, a non-ASCII CI gate, and registry sync validation.
2) `promote-monitor-lock` — add a promotion lock file so `clade-monitor` does not fight `clade-promote` during Canary boots.
3) `mesh-ui-integration` — land the pending mesh UI files (`MeshTabView.swift`, `clade-meshd/`, `MeshModels.swift`) behind feature flags and e2e seal.
