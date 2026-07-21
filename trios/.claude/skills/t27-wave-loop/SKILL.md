---
name: t27-wave-loop
description: T27 recurring wave loop for trios - audit weak spots, research literature, decompose, implement, verify, commit, save experience, report, and propose three next-wave options.
argument-hint: [domain] [context]
---

# T27 Wave Loop Skill (trios adaptation)

Use this skill when the user asks for a recurring `/loop Nm` macro that researches, plans, implements, and reports across multiple waves.

## Charter

1. **Audit weak spots** - run a security/robustness audit on the current trios subsystem.
2. **Literature search** - find 3-5 recent papers/tools relevant to the discovered weak spots.
3. **Decompose** - write a `.trinity/wave-loop-NNN.md` plan with P0-P5 priorities and file targets.
4. **Implement** - apply the plan spec-first using t27-creator or safe hand edits:
   - Write or update `.trinity/specs/*.md`.
   - Patch source/build/agent/skill files.
   - Keep L3 PURITY (ASCII-only) and L7 UNITY (no new `.sh` on the critical path).
5. **Verify** - run `./build.sh`, `cargo test --workspace`, `cargo clippy --workspace`, and t27-verifier.
6. **Commit** - stage, commit with Conventional Commit, push if requested.
7. **Save experience** - append `.trinity/experience.md` and write `.trinity/experience/YYYY-MM-DD_hh-mm-ss_WAVE-NNN.json`.
8. **Save skills** - update this skill and any new skills that capture reusable process.
9. **Report** - produce a wave closeout with status, artifacts, tests, and three future cooperation options.

## Output Format

At plan:

```
## T27 Wave Loop - Plan WAVE-{NNN}
Domain: {domain}
Context: {context}

P0 (Critical / must land now):
- {task} -> {files}

P1 (High / next wave):
- {task} -> {files}

P2 (Medium):
- {task} -> {files}

P3-P5 (Backlog / research):
- {task}

Literature takeaways:
- {paper 1}
- {paper 2}
- {paper 3}
```

At closeout:

```
## T27 Wave Loop - Closeout WAVE-{NNN}
Status: {SEALED|DRIFTED|TOXIC|PARTIAL}
Verified by: {build|tests|verifier}

Artifacts:
- Specs: .trinity/specs/{spec}.md
- Code: {files}
- Tests: {test results}
- Experience: .trinity/experience/YYYY-MM-DD_hh-mm-ss_WAVE-{NNN}.json
- Skills: .claude/skills/{skill}/SKILL.md

[FUTURE OPTIONS]
1) {next wave option 1}
2) {next wave option 2}
3) {next wave option 3}
```

## Rules

- Always produce a decomposed plan before implementation.
- Always produce at least three future options at closeout.
- Always save experience and skills before ending the wave.
- Never introduce a new `.sh` script on the critical path (L7 UNITY).
- Keep all source, specs, agents, and skills ASCII-only (L3 PURITY).
- If the BrowserOS Agent server is required for e2e seal and is down, record `E2E_BLOCKED_BY_SERVER_HEALTH` rather than failing the land.
