# CLAUDE.md — Instructions for Claude Code and autonomous agents (trios)

Use this file **together with** `[AGENTS.md](AGENTS.md)`. Repo-specific law always overrides generic tooling defaults.

---

## Autonomous Execution Loop (AEL v2.0)

When operating as the Trinity Agent (Queen), follow this 6-phase loop:

```
┌─────────────────────────────────────────────────────────────┐
│  OBSERVE → PLAN → DELEGATE → VERIFY → SYNTHESIZE → LEARN   │
│         ↓       ↓        ↓        ↓         ↓         ↓    │
│  [E]     [T]     [C/V]    [V]      [L]      [L]           │
└─────────────────────────────────────────────────────────────┘
```

### Phase 1: OBSERVE
- Call Experience Agent (E) for context — read `.trinity/experience.md` and `.trinity/experience/*.json`
- Check `.trinity/current_task/activity.md` for active task details
- Gather relevant files and context from trios Swift codebase
- Run `cargo run --bin clade-build` to establish baseline

### Phase 2: PLAN
- Break down task into subtasks
- Identify required skills: `/phi-loop`, `/tri-pipeline`, `/experience-save`
- Determine which agents to delegate to (see AGENTS.md alphabet)
- Estimate complexity and dependencies
- Select road from `.trinity/state/three-roads.json`:
  - **Road A** (fastest) — direct fix, minimal ceremony
  - **Road B** (balanced) — fix + test + experience save
  - **Road C** (deep) — spec-first, full PHI LOOP, agent spawn

### Phase 3: DELEGATE
- Delegate implementation to specialized agent (C)
- Delegate validation to Verifier Agent (V)
- Coordinate parallel execution where possible (max 3 agents simultaneous)
- Monitor agent progress via `.trinity/agent_events.jsonl`

### Phase 4: VERIFY
- Review agent outputs
- Run `cargo run --bin clade-build` — must pass
- Run `cargo run --bin clade-e2e` — must pass
- Check L1-L7 law compliance
- Ensure no regression in other tabs/features

### Phase 5: SYNTHESIZE
- Combine agent results
- Resolve conflicts (if two agents touched same file)
- Create cohesive solution
- Prepare for integration

### Phase 6: LEARN
- Call Learner Agent (L) for pattern extraction
- Update `.trinity/experience.md` via `/experience-save`
- Save ring-specific learnings as `.trinity/experience/YYYY-MM-DD_title.json`
- Improve future execution

---

## 1. Mandatory read order for this repository

1. `[AGENTS.md](AGENTS.md)` — entry point and constitutional stack.
2. `[.trinity/SOUL.md](.trinity/SOUL.md)` — canonical law (TDD, language, validation, T27 canon files).
3. `[.trinity/policy/coordination-law.md](.trinity/policy/coordination-law.md)` — shared-state mutation protocol (claims, queue, Akashic log).
4. `[docs/T27-CONSTITUTION.md](docs/T27-CONSTITUTION.md)` — T27 law port (L1-L7, DOCS-TREE, SSOT-MATH/SWIFT).
5. `[AGENTS.md](AGENTS.md)` — 27-agent alphabet and coordination rules.
6. `[.claude/agents/t27-queen.md](.claude/agents/t27-queen.md)` — T27 Queen AEL v2.0 orchestration.
7. `[.trinity/state/session_summary.md](.trinity/state/session_summary.md)` — what was built last.
8. `[.trinity/experience.md](.trinity/experience.md)` — prior learnings and mistakes.

---

## 2. Engineering workflow

- **Build:** `./build.sh` (swiftc direct compilation, no SPM/Xcode)
- **Run:** `open trios.app` (preferred — loads `Bundle.main` resources incl. the
  menu-bar logo). `./trios_app` works but the bare binary may not resolve bundle assets.
- **E2E:** `bash e2e/trios_e2e_flow.sh`
- **Health:** `curl -s http://127.0.0.1:9105/health`
- **Git:** branch `feat/zai-provider`, main branch is `dev`
- **T27 pipeline:** `/t27-phi-loop` or `/t27-tri-pipeline` for spec-first work on canon files

> **INVARIANT — menu-bar logo:** the trios status-bar logo must never disappear.
> It only vanishes when the **app process dies**. After any `./build.sh` /
> `clade-build` you MUST relaunch the app (`open trios.app`) — the running app
> otherwise keeps the old binary, and if it was killed the logo is gone until
> restarted. `clade-monitor`'s app watchdog relaunches it within ~60s as a
> backstop. See `.claude/rules/cron-life.md` → "INVARIANT: trios menu-bar logo".

---

## 3. PHI LOOP Execution

Follow the 9-phase PHI LOOP for ring-based development:

1. **Issue** — Define problem or requirement (GitHub issue #N)
2. **Spec** — Write agent instruction or skill spec
3. **TDD** — Define test criteria (build passes, e2e passes, no regressions)
4. **Code/Impl** — Implement in Swift according to spec
5. **Gen** — Not applicable (trios has no code generator; Swift is canonical)
6. **Seal** — Verify build and run e2e
7. **Verify** — Run tests, check UI anomalies
8. **Land** — Merge changes to `dev` branch
9. **Learn** — Capture learnings and update knowledge base

### Phase Completion Marker

When a phase is complete, include in your output:
```
Phase complete: [phase name]
→ Phase [next phase number]: [next phase name]
```

---

## 4. Autonomous subagent behavior (when spawned unattended)

- Finish the assigned task without waiting for clarification unless the repo's own rules require human input.
- If blocked after reasonable retries, stop and report what failed (logs, commands, file paths).
- Prefer small, reviewable diffs; match existing style and naming in touched files.
- **Output persistence:** when the parent workflow requires it, write the full final report to `/tmp/claude_code_output.md`.

---

## 5. Skills and tooling

### T27 Skills (spec-first / canon governance)

- `/t27-phi-loop` — T27 9-phase PHI LOOP adapted for trios
- `/t27-tri-pipeline` — `clade-build` → `clade-e2e` → `clade-seal` → `clade-promote`
- `/t27-experience-save` — Save episodes to `.trinity/experience/`
- `/t27-wave-loop` — Standing-wave charter for multi-variant work

### trios-Specific Skills

- `/phi-loop` — Execute 9-phase PHI LOOP
- `/tri-pipeline` — Execute tri commands (build, e2e, verify)
- `/experience-save` — Save learnings to persistent memory
- `/doctor` — Diagnose and heal build/dirty state
- `/god-mode` — Full oversight and audit
- `/bridge` — BrowserOS MCP bridge operations

Load these skills when their functionality matches the task.

---

## 6. Security and secrets

- Never commit secrets. Root `.env` patterns are gitignored; use `.env.example` patterns only in docs.
- The `.env` file in `trios-mcp-rag` contains LIVE credentials — never copy to trios.
- API keys (OpenRouter, etc.) are read from environment or `~/.trios/config.json`.

---

## The 7 Invariant Laws (trios adaptation)

| Law | Name | Description |
|------|------|-------------|
| L1 | TRACEABILITY | No code merged without `Closes #N` |
| L2 | GENERATION | Agent instructions/skills/specs are source of truth; canon Swift files (`BR-OUTPUT/`, selected `rings/`) are generated/reviewed artifacts; hand edits require Agent V waiver |
| L3 | PURITY | Source files ASCII-only with English identifiers |
| L4 | TESTABILITY | Every change must pass `./build.sh` + e2e flow + agent V verdict |
| L5 | IDENTITY | φ² = φ + 1; φ² + φ⁻² = 3; sacred constants in UI (GoldenFloat) |
| L6 | CEILING | `ProjectPaths.swift` + `TriosTheme.swift` are UI SSOT |
| L7 | UNITY | No new `*.sh` on critical path; use `build.sh` or MCP tools |

**Law Priority:** L1 > L2 > L3 > L4 > L5 > L6 > L7 (Asimov-style hierarchy)

---

**Repository:** trios — Swift macOS app for Trinity A2A network. **φ² + 1/φ² = 3 | TRINITY**
