# Wave Loop 002 — trios shell-safety, ASCII purity, and runtime-path hardening plan

## Sources
- Wave 2 safety audit by t27-creator.
- Literature scan:
  - [IsolateGPT: An Execution Isolation Architecture for LLM-Based Agentic Systems](https://arxiv.org/pdf/2403.04960)
  - [AgentBound: Securing Execution Boundaries of AI Agents](https://www.lucadigrazia.com/papers/fse2026.pdf)
  - [Towards Practically-Secure Tools for AI Agents](https://atlas.cs.brown.edu/pdf/haven:euromlsys:2026.pdf)
  - [Taming OpenClaw: Security Analysis and Mitigation of Autonomous LLM Agent Threats](https://www.arxiv.org/pdf/2603.11619)
  - [AgenTRIM: Tool Risk Mitigation for Agentic AI](https://arxiv.org/pdf/2601.12449)
  - [IterInject: Indirect Prompt Injection Against LLM Agents via Feedback-Guided Iterative Optimization](https://arxiv.org/html/2605.24659v1)
  - [ChatInject: Abusing Chat Templates for Prompt Injection in LLM Agents](https://arxiv.org/pdf/2509.22830)
  - [InjecAgent: Benchmarking Indirect Prompt Injections in Tool-Integrated Large Language Model Agents](https://arxiv.org/html/2403.02691v2)
  - [AgentDojo: A Dynamic Environment to Evaluate Prompt Injection Attacks and Defenses for LLM Agents](https://arxiv.org/html/2406.13352v3)
  - [Your Agent is More Brittle Than You Think: Uncovering Indirect Injection Vulnerabilities in Agentic LLMs](https://arxiv.org/html/2604.03870)

## Key research takeaways
1. Execution isolation via process-level sandboxing (IsolateGPT) maps to trios replacing `/bin/zsh -c` with tokenized `Process()` per tool.
2. MCP/agent capability manifests (AgentBound) justify a strict allowlist of commands instead of regex blocklists.
3. Static+dynamic effect analysis (Haven) supports adding a `CommandSanitizer` with data-flow validation.
4. Lifecycle threat models (OpenClaw) show execution-stage defenses are essential — trios TerminalTabView is a pure execution-stage surface.
5. IterInject/ChatInject prove regex blocklists are bypassable — trios must use allowlists and circuit breakers, not pattern matching.
6. RepE-based circuit breakers suggest adding an `AGENT-V-WAIVER` gate before any canon shell-related change.

## Decomposed plan (P0 -> P2)

### P0 — Critical shell-safety
- [ ] Replace `TerminalTabView.runCommand` shell invocation with tokenized `Process()` and strict command allowlist.
- [ ] Replace `QueenStatusViewModel.shell`/`shellAsync` helpers with tokenized `Process()` wrappers for `pgrep`, `ps`, `git`, `tail`.
- [ ] Add `CommandSanitizer` that rejects commands containing shell metacharacters before any Process spawn.
- [ ] Add promotion lock so `clade-promote` and `clade-monitor` cannot fight (future wave).

### P1 — ASCII purity and runtime paths
- [ ] Run ASCII cleanup over `BR-OUTPUT/*.swift`, `build.sh`, `.claude/agents/*.md`, `.claude/skills/*/*.md`.
- [ ] Move singleton lock/PID from `/tmp` to `.trinity/run/` and set `0o600` permissions.
- [ ] Move build logs from `/tmp` to `.trinity/logs/`.
- [ ] Fix `.claude/agents/registry.json` sync (missing `agent-H.md`).

### P2 — Tests and observability
- [ ] Add unit tests for `RecursionGuard`, `CladeGuard`, and command sanitizer.
- [ ] Add CI check that registry.json matches on-disk agent files.
- [ ] Standardize logging to `.trinity/logs/` with component/correlation IDs.

## This iteration goal
Land focused P0/P1 items that do not require UI testing:
1. ASCII-only cleanup of `BR-OUTPUT/*.swift`.
2. Move singleton lock/PID to project-relative `.trinity/run/` (updates `ProjectPaths.swift` and `RecursionGuard.swift`).
3. Replace `QueenStatusViewModel.shell` with tokenized `Process()` for health probes.
4. Fix registry.json sync.
5. A spec file for each change.
6. Verifier verdict and experience save.

## [FUTURE OPTIONS]
1) `terminal-shell-free` — fully replace `TerminalTabView.runCommand` `/bin/zsh -c` with a command allowlist + tokenized Process.
2) `promote-monitor-lock` — add a promotion lock file that `clade-monitor` respects during `clade-promote` boot probe.
3) `asciify-all-the-things` — complete ASCII cleanup of `.claude/agents/*.md` and `.claude/skills/*/*.md` and add a CI lint gate.
