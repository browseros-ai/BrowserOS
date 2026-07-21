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

## 2026-07-21 WAVE-001 (Kernel/Safety)

- **Issue**: #T27-EPIC-001
- **Agents**: t27-creator, t27-verifier
- **Root cause**: trios-mesh was exempt from workspace unwrap_used lint, hiding panic surfaces; CladeGuard rollback removed the binary before copying, and verifyChecksum accepted snapshots with missing checksums.
- **Fix pattern**: Add [lints] workspace = true to trios-mesh and cfg_attr test exemption; replace NaN-sensitive partial_cmp unwraps with total order; rewrite CladeGuard applySnapshot to use NSFileCoordinator + replaceItemAt atomic swap; make verifyChecksum fail closed on missing sidecar.
- **Files changed**: trios/rings/RUST-13/trios-mesh/Cargo.toml, trios/rings/RUST-13/trios-mesh/src/lib.rs, trios/rings/RUST-13/trios-mesh/src/router.rs, trios/rings/RUST-13/trios-mesh/src/routing.rs, trios/rings/RUST-13/trios-mesh/build.rs, trios/BR-OUTPUT/CladeGuard.swift, trios/.trinity/specs/trios-mesh-lints.md, trios/.trinity/specs/clade-guard.md, trios/.trinity/wave-loop-001.md
- **Tests added**: trios-mesh existing test suite (101 tests) continues to pass, clade-tablecloth flaky throttle test passed on retry
- **Lessons**:
  - Nested git repos (trios-mesh) must be committed inside the submodule first; parent repo only sees the pointer update.
  - Workspace-wide lints can suddenly expose debt in one crate; gate the lint addition with targeted test exemptions plus a plan to clean production expects.
  - Atomic file replacement on macOS should use FileManager.replaceItemAt inside an NSFileCoordinator, not remove-then-copy.
  - A verifier agent must be spawned per wave to keep L2 GENERATION and L4 TESTABILITY honest.
- **Seal status**: BUILD_PASS, TEST_PASS, CLIPPY_PASS, E2E_NOT_RUN_DUE_SERVER_DOWN

## 2026-07-21 WAVE-002 (Safety/Hardening)

- **Issue**: #T27-EPIC-001
- **Agents**: t27-creator, t27-verifier
- **Root cause**: BR-OUTPUT Swift files violated L3 PURITY with non-ASCII characters; QueenStatusViewModel used /bin/zsh -c for health probes creating CWE-78 shell injection surface; singleton lock lived in world-writable /tmp; registry.json referenced a missing agent file.
- **Fix pattern**: Batch-replace non-ASCII chars in BR-OUTPUT with ASCII equivalents per ascii-cleanup.md. Add run/runAsync tokenized Process helpers to QueenStatusViewModel and migrate all health probes. Move singleton lock/PID to .trinity/run/ with restricted perms. Remove agent-H from registry.json.
- **Files changed**: trios/BR-OUTPUT/BrowserOSChatViewModel.swift, trios/BR-OUTPUT/ChatLogic.swift, trios/BR-OUTPUT/ChatPanelView.swift, trios/BR-OUTPUT/GitButlerViewModel.swift, trios/BR-OUTPUT/LLMClient.swift, trios/BR-OUTPUT/MessageBubbleView.swift, trios/BR-OUTPUT/MeshTabView.swift, trios/BR-OUTPUT/ProjectPaths.swift, trios/BR-OUTPUT/QueenStatusBadge.swift, trios/BR-OUTPUT/QueenStatusViewModel.swift, trios/BR-OUTPUT/QueenTabView.swift, trios/BR-OUTPUT/RecursionGuard.swift, trios/BR-OUTPUT/RichTextRenderer.swift, trios/BR-OUTPUT/TerminalTabView.swift, trios/BR-OUTPUT/TriosMCPClient.swift, trios/BR-OUTPUT/WindowManager.swift, trios/.claude/agents/registry.json, trios/.trinity/specs/ascii-cleanup.md, trios/.trinity/specs/singleton-lock-paths.md, trios/.trinity/specs/queen-shell-free.md, trios/.trinity/specs/agent-registry-sync.md, trios/.trinity/wave-loop-002.md
- **Tests added**: ASCII scan over BR-OUTPUT/*.swift, grep for shellAsync/shell( in QueenStatusViewModel, registry.json validation script
- **Lessons**:
  - ASCII-only policy is enforceable with a single Python scan; batch replacement preserves semantics if done carefully.
  - Shell-free Process helpers dramatically reduce attack surface but require careful async actor crossing in @MainActor Swift.
  - Singleton lock path must be user-private; /tmp is unsafe for process identity.
  - Registry drift (missing agent-H) is a latent L1 TRACEABILITY bug; add CI validation.
- **Seal status**: BUILD_PASS, TEST_PASS, CLIPPY_PASS, E2E_NOT_RUN_DUE_SERVER_DOWN

## 2026-07-21 WAVE-003 (Shell-free / Portable / ASCII)

- **Issue**: #T27-EPIC-001
- **Agents**: t27-creator, t27-verifier, t27-experience
- **Root cause**: TerminalTabView still used `/bin/zsh -c` for arbitrary commands; clade-build and build.sh hardcoded `/Users/playra/BrowserOS-full/trios`; agents and skills contained emoji, arrows, and em-dashes that violated L3 PURITY.
- **Fix pattern**: Rewrite TerminalTabView with `TerminalCommandSanitizer.sanitize()` producing tokenized `Process()` requests. Make clade-build derive its root from `TRIOS_ROOT` with `current_dir()` fallback and move logs to `.trinity/logs/`. ASCII-clean all `.claude/agents/*.md` and `.claude/skills/*/*.md`. Update `t27-wave-loop/SKILL.md` and create `ascii-lint/SKILL.md`.
- **Files changed**: trios/BR-OUTPUT/TerminalTabView.swift, trios/build.sh, trios/rings/RUST-01/clade-build/src/main.rs, trios/.trinity/specs/terminal-shell-free.md, trios/.trinity/specs/build-cleanup.md, trios/.claude/skills/t27-wave-loop/SKILL.md, trios/.claude/skills/ascii-lint/SKILL.md, trios/.claude/agents/*.md, trios/.claude/skills/*/*.md
- **Tests added**: `./build.sh`, `cargo test --workspace`, `cargo clippy -p clade-build --all-targets --all-features`, ASCII scan over source/agents/skills
- **Lessons**:
  - Shell-free dispatch is enforceable with a small sanitizer: split on space, allowlist executable, reject shell metacharacters.
  - Removing hardcoded paths from build tooling lets the repo be checked out anywhere; fall back to `current_dir()` when `TRIOS_ROOT` is unset.
  - Agent and skill markdown must be ASCII-only too; a bulk transliterator can preserve meaning while satisfying the lint.
  - Saving skills at the end of a wave turns one-off cleanup into reusable institutional memory.
- **Seal status**: BUILD_PASS, TEST_PASS, CLIPPY_PASS, E2E_NOT_RUN_DUE_SERVER_DOWN
