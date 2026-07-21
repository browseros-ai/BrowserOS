# Plan: T27 Automation Refactor for trios - Seal Manual Swift Code Behind Agents

**Date:** 2026-07-21
**Issue target:** #TBD - create after plan approval
**Branch:** `feat/t27-automation`
**Status:** Draft - pending approval

> Goal: migrate all hand-written BR-OUTPUT Swift code behind a T27 agent-driven generation/verification pipeline, port t27 language-spec + skills + laws into trios, and enforce the rule: **no direct hand edits to generated/canon Swift; every change flows through Agent T (Queen) -> Creator C -> Verifier V -> Experience E.**

---

## 1. Goal & Definition of Done

### Primary Goal
Transform trios from a hand-coded Swift macOS app into a **T27-governed system** where:

1. **Agent instructions and `.t27`-style specs are the single source of truth** for BR-OUTPUT behavior.
2. **Swift source in `BR-OUTPUT/` is treated as generated/canon output** - hand-edits require an agent review stamp (Agent V).
3. **A T27 agent lattice** (Creator C, Verifier V, Learner L, Queen T, Experience E) lives in `trios/.claude/agents/` and is invoked by skills.
4. **The autonomous loop (`clade-monitor`) can spawn T27 agents** for planned refactoring tasks, not only health/build checks.
5. **t27 laws, skills, hooks, and coordination protocol** are ported and adapted for a Swift/Rust codebase.

### Definition of Done

- [ ] `trios/.claude/agents/t27-creator.md`, `t27-verifier.md`, `t27-learner.md`, `t27-queen.md`, `t27-experience.md` created and registered.
- [ ] `trios/.claude/skills/t27-phi-loop/SKILL.md`, `t27-tri-pipeline/SKILL.md`, `t27-experience-save/SKILL.md`, `t27-wave-loop/SKILL.md` created.
- [ ] `trios/.trinity/policy/coordination-law.md` and `.trinity/queue/` ported from t27.
- [ ] A **pilot component** (`RecursionGuard.swift` or `ChatLogic.swift`) is fully spec-driven: spec -> agent implementation -> tests -> seal -> no hand edits allowed.
- [ ] L2 GENERATION law updated in `trios/.trinity/SOUL.md` and `trios/CLAUDE.md` to cover Swift canon files.
- [ ] L7 UNITY clarification: `build.sh` and `e2e/trios_e2e_flow.sh` are grandfathered; all *new* build/test logic must be Rust rings or agent skills.
- [ ] `clade-monitor` extended with a T27 task scheduler that can enqueue agent work into `.trinity/queue/`.
- [ ] All changes committed with `Closes #TBD`.

---

## 2. Context: Why This Refactor Now

### Current State
- `trios/BR-OUTPUT/` contains 34 hand-written Swift files (ViewModels, Views, helpers).
- Agents exist (`agent-A..Z`, `queen-browseros`, `tri-orchestrator`) but most alphabet agents are stubs; only queen-browseros is fully active.
- Skills exist but are trios-specific (`/tri`, `/phi-loop`, `/clade-seal`, `/doctor`); no T27 generic spec-first skills.
- The recursive self-launch fix (commit `6036a495`) was done by hand and demonstrates the risk: critical safety code lives in `BR-OUTPUT/RecursionGuard.swift` with no spec/agent provenance.

### t27 Assets to Port
From `/Users/playra/t27`:
- Constitutional stack: `SOUL.md`, `CLAUDE.md`, `docs/T27-CONSTITUTION.md`.
- Agents: `creator.md`, `verifier.md`, `learner.md`, `experience.md`, `trinity.md`.
- Skills: `phi-loop.md`, `tri-pipeline.md`, `experience-save.md`, `t27-wave-loop.md`, `gitbutler-commit.md`.
- Coordination: `.trinity/policy/coordination-law.md`, `.trinity/queue/`, `.trinity/events/akashic-log.jsonl`.
- Hooks: `.claude/hooks/check-l1-traceability.sh`, `.claude/hooks/stop-hook-guard.sh`.
- Git hooks: `.githooks/pre-commit`, `.git/hooks/commit-msg`, `.git/hooks/pre-commit`.

### Strategic Value
- **Safety:** agent-reviewed canon files reduce accidental breakage of invariants (menu-bar logo, recursion guard, etc.).
- **Scale:** 27 agents can own 27 areas of trios instead of one queen-browseros doing everything.
- **Traceability:** every change links to a spec and an issue (L1).
- **Self-improvement:** T27 loop can propose its own refactors, subject to safety budget.

---

## 3. Decomposed Work Breakdown

### Phase 0: Foundation & Constitution (P0)
*Goal: establish T27 law in trios before touching code.*

#### 0.1 Update `trios/.trinity/SOUL.md`
- Add Article L2-GENERATION-SWIFT: `BR-OUTPUT/` and selected `rings/SR-02/` files are **canon/generated**; hand edits require Agent V stamp.
- Add Article T27-COORDINATION: before mutating shared state, read Akashic log, claims, queue; acquire exclusive claim with TTL.
- Preserve existing ASCII-only, TDD, NO-NEW-SHELL rules.

#### 0.2 Update `trios/CLAUDE.md`
- Add mandatory read order: `AGENTS.md` -> `SOUL.md` -> `.trinity/policy/coordination-law.md` -> `TASK.md`/`TASK_PROTOCOL.md`.
- Port AEL v2.0 OBSERVE->PLAN->DELEGATE->VERIFY->SYNTHESIZE->LEARN.
- Port 9-phase PHI LOOP marker `Phase complete: [phase]` -> `-> Phase [N]: [name]`.
- Clarify L7 UNITY: `build.sh` and `e2e/trios_e2e_flow.sh` are pre-existing exceptions; new critical-path tooling must be Rust rings or agent skills.

#### 0.3 Create `trios/AGENTS.md` delta
- Map existing 27 alphabet agents to T27 roles (A=Architect, C=Compiler/Swift generator, V=Verifier, etc.).
- Add T27 agents section: Creator C, Verifier V, Learner L, Queen T, Experience E.
- Document that `queen-browseros` delegates to T27 agents for spec-first work.

#### 0.4 Create `trios/.trinity/policy/coordination-law.md`
- Port from `/Users/playra/t27/.trinity/policy/coordination-law.md`.
- Adapt file paths from `t27/.trinity/` to `trios/.trinity/`.
- Define claim TTL, heartbeat, Akashic log schema, loop handoff with 3 future options.

#### 0.5 Create `trios/.trinity/queue/` structure
- `active.json`, `pending.json`, `blocked.json`, `done.json`.
- JSON schema: id, title, agent, phase, claim_until, heartbeat_ts, dependencies, issue_url.

---

### Phase 1: T27 Agent Lattice (P0)
*Goal: introduce T27 agents alongside existing trios agents.*

#### 1.1 Create `trios/.claude/agents/t27-queen.md`
- Port `/Users/playra/t27/.claude/agents/trinity.md` + `.trinity/agents/AGENT_T_SKILL.md`.
- Queen T owns the AEL v2.0 loop for trios.
- Reads `.trinity/current-issue.md`, `queue/`, `experience.md`.
- Delegates to C/V/L/E.
- Reports with 6-phase status and next road A/B/C.

#### 1.2 Create `trios/.claude/agents/t27-creator.md`
- Port `/Users/playra/t27/.claude/agents/creator.md` + `agent-c-compiler.md`.
- Adapt to Swift: does **not** compile `.t27`, but follows a Swift spec to generate/edit canon Swift files.
- Rules: spec-first, never hand-edit canon without V, ASCII-only, tests included.
- Tools: Read, Edit, Write, Bash (swiftc, build.sh), Agent (delegate to V).

#### 1.3 Create `trios/.claude/agents/t27-verifier.md`
- Port `/Users/playra/t27/.claude/agents/verifier.md` + `agent-v-verify.md`.
- Checks L1-L7 compliance for any proposed change.
- Runs `./build.sh`, `cargo test`, `clade-e2e` where relevant.
- Blocks land if violations found; writes verdict to `.trinity/state/verdicts/`.

#### 1.4 Create `trios/.claude/agents/t27-experience.md`
- Port `/Users/playra/t27/.claude/agents/experience.md` + `agent-e-experience.md`.
- Reads `.trinity/experience.md` and `mistakes-catalog.json`.
- Suggests similar solved episodes before implementation.
- Writes new episodes after land.

#### 1.5 Create `trios/.claude/agents/t27-learner.md`
- Port `/Users/playra/t27/.claude/agents/learner.md` + `agent-l-lsp.md`.
- Extracts patterns from completed work.
- Updates `ring-{NNN}.md` and `.trinity/experience.md`.
- Can propose agent instruction updates.

#### 1.6 Register agents in `trios/.claude/agents/registry.json`
- If registry exists, append T27 agents.
- Otherwise create it mapping agent name -> file -> domain -> default skill.

---

### Phase 2: T27 Skills (P0-P1)
*Goal: provide invocable skills that drive the T27 loop.*

#### 2.1 `trios/.claude/skills/t27-phi-loop/SKILL.md`
- Port `/Users/playra/t27/.claude/skills/phi-loop.md`.
- 9 phases adapted for trios: Issue -> Spec -> TDD -> Code/Impl -> **Review** (no gen) -> Seal -> Verify -> Land -> Learn.
- Phase completion marker triggers automatic queue update.
- Roads: A (critical hotfix), B (standard ring dev), B-clade (Canary), C (deep spec-first).

#### 2.2 `trios/.claude/skills/t27-tri-pipeline/SKILL.md`
- Port `/Users/playra/t27/.claude/skills/tri-pipeline.md`.
- Pipeline for trios: `clade-build` -> `clade-e2e` -> `clade-seal` -> `clade-promote`.
- Replaces `tri gen` with Swift compilation; keeps TDD/seal/verify semantics.

#### 2.3 `trios/.claude/skills/t27-experience-save/SKILL.md`
- Port `/Users/playra/t27/.claude/skills/experience-save.md`.
- Saves episode to `.trinity/experience/episodes/` and appends summary to `.trinity/experience.md`.
- Includes issue #, root cause, fix pattern, agent chain.

#### 2.4 `trios/.claude/skills/t27-wave-loop/SKILL.md`
- Port `/Users/playra/t27/.claude/skills/t27-wave-loop.md`.
- Standing-wave charter for trios: explore, plan 2-4 variants, implement one, closeout with 3 future options.

#### 2.5 Update existing skills to reference T27
- `phi-loop/SKILL.md`: add note that for deep/spec-first work use `/t27-phi-loop`.
- `clade-seal/SKILL.md`: add Agent V verdict gate.
- `doctor/SKILL.md`: route architectural changes through T27 Queen.

---

### Phase 3: Hooks & Git Automation (P1)
*Goal: enforce T27 law mechanically.*

#### 3.1 `trios/.claude/hooks/check-l1-traceability.sh`
- Port from t27; forward L1 check to a Rust ring or grep.
- Trigger: `PreToolUse` before Write/Edit that touches canon files (`BR-OUTPUT/`, `rings/SR-02/`).
- Block if no issue link in `.trinity/current-issue.md` or branch name.

#### 3.2 `trios/.claude/hooks/stop-hook-guard.sh`
- Port from t27.
- On session stop, save `phi-loop.json`, log uncommitted canon changes, release claims.

#### 3.3 `trios/.claude/settings.json`
- Register hooks and whitelist allowed commands (build.sh, cargo, swiftc, open).

#### 3.4 Git hooks in `trios/.githooks/` and `trios/.git/hooks/`
- `pre-commit`: L3 PURITY (ASCII-only) + NOW.md/activity.md gate.
- `commit-msg`: L1 TRACEABILITY (`Closes #N` / `Fixes #N`).
- Optional `pre-push`: require `.trinity/current-issue.md` existence (NotebookLM gate removed or adapted).

#### 3.5 Update root `lefthook.yml` interaction
- Ensure trios-specific hooks do not conflict with root `lefthook.yml` Biome/file-length checks.
- Document: from repo root only root hooks run; trios T27 hooks run when `cwd=trios` or via Claude settings.

---

### Phase 4: Pilot - Spec-Driven `RecursionGuard` (P1)
*Goal: prove the T27 pipeline on a single, safety-critical component.*

#### 4.1 Create spec
- File: `trios/.trinity/specs/SR-00-recursion-guard.t27` (or `.md` if `.t27` parser unavailable).
- Define invariants:
  - Only one trios process may hold user-visible UI.
  - Bare binary and `.app` launches converge to one instance.
  - Existing instance is activated, not killed.
  - Lock must auto-release on crash.
  - Health endpoint considered as secondary alive signal.

#### 4.2 TDD phase
- Extend `tests/swift/chat_logic_test.swift` if relevant.
- Add Rust unit tests for `clade-monitor` watchdog detection.
- Add manual test script in skill form, not `.sh`.

#### 4.3 Agent C generates/updates `BR-OUTPUT/RecursionGuard.swift`
- Must follow spec, ASCII-only, no hardcoded paths.
- If changes needed, Agent C proposes, Agent V verifies.

#### 4.4 Agent V verifies
- Run `./build.sh`, `cargo test`, launch `trios.app` twice, verify 1 process.
- Check L1-L7.
- Write verdict.

#### 4.5 Seal & Land
- `/clade-seal` on staging.
- `/clade-promote` if e-value >= 5.
- Commit: `ring-SR-00-seal: RecursionGuard T27 spec-driven (Closes #TBD)`.

#### 4.6 Learn
- `/experience-save` writes episode about recursion-guard T27 migration.

---

### Phase 5: Queue Integration in `clade-monitor` (P2)
*Goal: the autonomous loop can pick up and schedule T27 work.*

#### 5.1 Extend `clade-monitor` with `t27_scheduler` module
- Every 15m health check also scans `.trinity/queue/pending.json`.
- If task is ready (dependencies done, claim free), acquire claim and spawn appropriate agent via `claude` CLI or MCP.
- Update `active.json` / `blocked.json` / `done.json`.

#### 5.2 Safety budget integration
- T27 tasks consume safety budget like `clade-tablecloth`.
- Halts if budget <= 0.

#### 5.3 Logging
- All scheduling decisions append to `.trinity/event_log.jsonl` and `akashic-log.jsonl`.

---

### Phase 6: Expand to All BR-OUTPUT Swift (P2-P3)
*Goal: every BR-OUTPUT file has a spec and provenance.*

#### 6.1 Categorize files by domain/agent
| File | Owner Agent | Spec location |
|---|---|---|
| RecursionGuard.swift | K (Kernel) + V | `.trinity/specs/SR-00-recursion-guard.*` |
| ChatLogic.swift | L (Language) + X (MCP) | `.trinity/specs/SR-02-chat-logic.*` |
| ChatPanelView.swift | H (UI) | `.trinity/specs/BR-OUTPUT-chat-panel.*` |
| BrowserOSChatViewModel.swift | X (External/MCP) | `.trinity/specs/SR-02-browseros-chat.*` |
| TriosMCPClient.swift | X | `.trinity/specs/SR-01-mcp-client.*` |
| QueenStatusViewModel.swift | T (Queen) | `.trinity/specs/SR-02-queen-vm.*` |
| ProjectPaths.swift | A (Architect) | `.trinity/specs/SR-00-project-paths.*` |
| TriosTheme.swift | H (UI) + P (Physics/phi) | `.trinity/specs/SR-00-theme.*` |

#### 6.2 Convert one file per wave loop
- Each wave loop migrates 1-3 related files.
- Follows T27 pipeline: spec -> TDD -> code -> review -> seal -> verify -> land -> learn.

#### 6.3 Freeze migrated files
- Add `.trinity/seals/{component}.json` after successful seal.
- Hand edits blocked by L2; changes must update spec and regenerate.

---

### Phase 7: Rust Rings Governance (P3)
*Goal: apply T27 discipline to Rust rings too.*

#### 7.1 Specs for critical Rust rings
- `clade-monitor`: watchdog, scheduling, safety budget.
- `clade-build`: variant resolution, Info.plist generation.
- `clade-audit`: 8 checks.
- `clade-promote`: atomic swap, boot probe.

#### 7.2 Agent ownership
- Agent C (Compiler/Swift) can also review Rust when no dedicated Rust agent exists; eventually spawn `agent-R` (Runtime/Rust).
- Agent V runs `cargo clippy`, `cargo test --workspace`.

---

### Phase 8: Documentation & Memory (P3-P4)
*Goal: humans and agents know the new rules.*

#### 8.1 Update `trios/README.md`
- Replace stale trinity-s3ai content with actual trios overview.
- Add T27 section: how agents own code, how to invoke `/t27-phi-loop`.

#### 8.2 Update `trios/LAUNCH.md`
- Document T27 launch flow: Queen T -> check queue -> run skills.

#### 8.3 Create `trios/docs/T27-CONSTITUTION.md`
- Adapted from t27 with Swift/Rust specifics.
- L1-L7 table, coordination law, spec-first pipeline.

#### 8.4 Memory files
- Save project memory: `trios-t27-automation-roadmap.md`.
- Update `~/.claude/projects/-Users-playra-BrowserOS-full/memory/MEMORY.md` index.

---

## 4. Risk Register

| Risk | Mitigation |
|---|---|
| t27 `.t27` parser cannot be reused for Swift specs | Use `.md` specs + structured YAML frontmatter until a Swift-spec parser exists. |
| Existing agents conflict with new T27 agents | Keep old agents for trios-specific tasks; T27 agents handle spec-first canon changes. Update `tri-orchestrator` routing table. |
| Build time increases due to agent review gates | Add fast-path for critical hotfixes (Road A) with post-hoc review. |
| Queue/claim system adds complexity | Start with simple JSON queue; evolve to SQLite/queue ring only if proven necessary. |
| Hand-edit freeze blocks urgent fixes | Allow emergency override with explicit `AGENT-V-WAIVER` comment and follow-up seal. |
| Root repo lefthook conflicts with trios T27 hooks | Document which hooks run where; keep pre-commit checks orthogonal. |
| `clade-monitor` cannot reliably spawn Claude agents headless | Fallback to logging queued tasks and letting user or cron launch them. |

---

## 5. Milestones & Estimates

| Milestone | Deliverables | Priority | Owner Agent |
|---|---|---|---|
| M1: Constitution ported | SOUL.md, CLAUDE.md, coordination-law.md, queue/ | P0 | A + T |
| M2: T27 agents live | 5 agent files + registry | P0 | T |
| M3: T27 skills live | 4 skill directories | P0-P1 | T + Z |
| M4: Hooks enforce law | check-l1, stop-hook, git hooks | P1 | V + W |
| M5: Pilot sealed | RecursionGuard spec-driven + seal | P1 | K + C + V |
| M6: Monitor scheduler | clade-monitor reads/writes queue | P2 | Q + O |
| M7: BR-OUTPUT migrated | All 34 files under T27 specs | P2-P3 | H, X, T, etc. |
| M8: Rust rings governed | clade-* specs + agent ownership | P3 | R + V |
| M9: Docs & memory | README, T27-CONSTITUTION, memory | P3-P4 | Z + E |

---

## 6. First Concrete Steps (after plan approval)

1. Create GitHub issue `#TBD` for this refactor.
2. Branch `feat/t27-automation` from `dev`.
3. Run Phase 0: update `SOUL.md`, `CLAUDE.md`, create `coordination-law.md` + `queue/`.
4. Run Phase 1: create 5 T27 agent files.
5. Run Phase 2: create 4 T27 skill directories.
6. Open PR for Phase 0-2; land with `Closes #TBD`.
7. Then proceed to Phase 3 (hooks) and Phase 4 (RecursionGuard pilot).

---

*phi^2 + 1/phi^2 = 3 | TRINITY*
