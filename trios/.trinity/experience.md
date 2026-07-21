# Trinity Experience Log — trios project

## 2026-05-24 — Queen BrowserOS Awakening
- Event: Full agent infrastructure deployed
- Agents created: queen-browseros.md
- Skills created: tri, doctor, god-mode, bridge
- MCP access: fs_read, fs_write, shell_execute confirmed working
- Build system: build.sh created, swiftc compilation successful
- Access path: BrowserOS-Agent -> Browser -> http://127.0.0.1:9105/mcp -> BrowserOS MCP -> Mac

## t27 Laws Applied
1. Skills First — all skills auto-invoke before action
2. Wrap-up MANDATORY — session memory preservation
3. Proactive Orchestration — detect, plan, execute, report

## Architecture
- Core: ChatMessage, AgentIdentity, ChatEvents (SR-00)
- Infrastructure: SSETransport, HealthCheckTransport (SR-01)
- Application: ChatViewModel, ConversationStateMachine (SR-02)
- Presentation: ChatPanelView, GlassmorphismBackground (BR-OUTPUT)
- Server: BrowserOS MCP on port 9105
- A2A: Registry endpoint for agent discovery

## Critical Learnings (2026-05-28)

### 1. Chat Input Fix — NSTextView + First Responder
**Ring:** BR-OUTPUT  **Agents:** T, H, K  **Road:** A
- **Problem:** SwiftUI TextField in NSPanel completely non-functional (no type, paste, focus)
- **Root cause:** NSHostingView doesn't retain NSHostingController (weak ref crash). NSTextField wrong for multi-line chat.
- **Fix:** NSTextView via NSViewRepresentable, remove weak from hostingController, explicit makeFirstResponder
- **Files:** `ChatPanelView.swift`, `WindowManager.swift`
- **Episode:** `.trinity/experience/2026-05-28_chat_input_nstextview.json`

### 2. State Machine Retry — Allow .error → .streaming
**Ring:** SR-02  **Agents:** T, R, Q  **Road:** A
- **Problem:** After timeout, all subsequent messages silently dropped
- **Root cause:** ConversationStateMachine blocked .error → .streaming transition
- **Fix:** Added .error → .streaming to canTransition()
- **Episode:** `.trinity/experience/2026-05-28_state_machine_retry.json`

### 3. SSE Manual Buffer — Don't Trust bytes.lines
**Ring:** SR-01  **Agents:** T, X  **Road:** A
- **Problem:** SSE stream silently hung, "The request timed out"
- **Root cause:** AsyncSequence.bytes.lines hung on certain chunk boundaries
- **Fix:** Manual Data buffer + newline parsing
- **Episode:** `.trinity/experience/2026-05-28_sse_manual_buffer.json`

### 4. Command Injection — Strict Prefix Matching
**Ring:** SR-02  **Agents:** T, X, V  **Road:** A
- **Problem:** Innocent messages like "swift is great" executed as shell commands
- **Root cause:** isLikelyCommand used fuzzy contains() matching; parseIntent fell through to shell
- **Fix:** Strict prefix only ("shell ", "run ", "exec ", "/"); return nil for unrecognized
- **Episode:** `.trinity/experience/2026-05-28_command_injection_fix.json`

### 5. Scroll Geometry — Content Height vs Viewport Height
**Ring:** BR-OUTPUT  **Agents:** T, H  **Road:** B
- **Problem:** Auto-scroll never fired for long conversations
- **Root cause:** Used viewport height instead of scroll content height in isNearBottom math
- **Fix:** ScrollContentHeightPreferenceKey with GeometryReader inside LazyVStack
- **Episode:** `.trinity/experience/2026-05-28_scroll_content_height.json`

### 6. Swift 6 Concurrency — Nonisolated Parsers
**Ring:** SR-02  **Agents:** T, R, V  **Road:** B
- **Problem:** A2ARegistryClient data race under strict concurrency
- **Root cause:** Actor-isolated mutable decoder accessed from AsyncStream Task
- **Fix:** parseSSELine made nonisolated with local decoder; static ISO8601DateFormatter
- **Episode:** `.trinity/experience/2026-05-28_a2a_concurrency_fix.json`

## Trinity Protocols Ported (2026-05-28)
- AEL v2.0 loop → `CLAUDE.md`
- PHI LOOP 9-phase → `.claude/skills/phi-loop/SKILL.md`
- 7 Invariant Laws (L1-L7) → `CLAUDE.md` + `.trinity/SOUL.md`
- 27-Agent Alphabet → `AGENTS.md` + `.trinity/agents/registry.json`
- 3-Roads Planning → `.trinity/state/three-roads.json`
- Experience Save → `.claude/skills/experience-save/SKILL.md`
- Mistakes Catalog (MNL) → `.trinity/experience/mistakes-catalog.json`
- Akashic Log Schema → `.trinity/events/akashic-log-schema.json`

## Key Decisions
- Flat swiftc compilation (no SPM/Xcode)
- Onion ring architecture (Core -> Infra -> App -> UI)
- Tailscale for remote access
- BR-OUTPUT/ for new UI components
- .claude/ for agent/skill definitions
- .trinity/ for experience, state, and constitutional law
## 2026-07-21 RECURSION-001 (Kernel)

- **Issue**: #T27-EPIC-001
- **Agents**: t27-creator, t27-verifier
- **Root cause**: trios had layered single-instance failures: missing Info.plist bundle ID prevented NSRunningApplication activation, PID file was written after a window race, pgrep -x detection was unreliable, and bare-binary launch bypassed bundle checks.
- **Fix pattern**: Centralize singleton paths in ProjectPaths.swift; acquire POSIX flock before writing PID with retries; detect existing instance via NSRunningApplication bundle ID with comm/args fallback; generate Info.plist in build.sh; block bare-binary launch. Also made clade-worktree tests deterministic by parameterizing env-dependent helpers instead of mutating global TRIOS_ROOT.
- **Files changed**: trios/BR-OUTPUT/RecursionGuard.swift, trios/BR-OUTPUT/ProjectPaths.swift, trios/build.sh, trios/rings/RUST-10/clade-worktree/src/main.rs, trios/.trinity/specs/recursion-guard.md
- **Tests added**: updated rings/RUST-10/clade-worktree tests to use parameterized helpers
- **Lessons**:
  - Canon Swift files must be spec-driven; the .md spec is SSOT and .swift is a derived artifact.
  - Workspace tests must not mutate global env; use parameterized helpers to stay deterministic under parallel execution.
  - ASCII-only policy applies to specs, policy, agent instructions, skills, and changed source lines.
  - External BrowserOS server health can block e2e seal; record the dependency and rerun seal when the server is up.
- **Seal status**: BUILD_PASS, TEST_PASS, E2E_BLOCKED_BY_SERVER_HEALTH
