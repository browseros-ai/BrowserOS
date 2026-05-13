# ACP Agent Mode — Session Management Spec

Date: 2026-05-12
Status: DRAFT v3 (frontend-only refocus)
Mode: **Agent Mode** (agent-command, harness, external agents)
Scope: **Frontend-only** ACP session UX for agent-command

**Scope lock**: Backend/server/protocol changes are deferred.
See: `docs/superpowers/specs/2026-05-12-session-management-backend-deferred.md`

---

## 1. Current State — Agent Mode

### What Exists

| Feature | File | Status |
|---------|------|--------|
| Agent CRUD (create/delete/config) | `agents.ts`, `AgentsPage.tsx` | ✅ |
| Agent conversation (harness) | `AgentCommandConversation.tsx` | ✅ |
| Harness chat history (hardcoded `'main'`) | `agent-harness-service.ts` L765-777 | ✅ |
| Queue (pending messages) | `QueuePanel.tsx` | ✅ |
| Outputs rail | `agent-conversation.outputs-rail.tsx` | ✅ |
| Cancel turn | `cancelHarnessTurn()` | ✅ |
| Active turn registry (per agent) | `active-turn-registry.ts` | ✅ |
| Agent definitions DB (Drizzle) | `lib/db/schema/agents.ts` | ✅ |

### Critical Existing Constraint: `sessionId: 'main'`

The entire harness stack hardcodes `sessionId: 'main'`:

```
agents.ts L328,409,417,655        → X-Session-Id: 'main'
agent-harness-service.ts L299,365,368,454,504,765,777,889,894,954,1023
                                  → sessionId: 'main' everywhere
active-turn-registry.ts           → registry keyed by (agentId, sessionId='main')
acpx-runtime.ts                   → getHistory/getRowSnapshot take sessionId
```

**The primary task is generalizing from single `'main'` to multi-session** — not building from zero.

### What's Missing

| Feature | Zed Equivalent | Priority |
|---------|----------------|----------|
| Multi-session per agent (beyond `'main'`) | `AgentConnection.new_session()` | P0 |
| Session list per agent | `AgentSessionList.list_sessions()` | P0 |
| Session search | cursor-based search | P0 |
| Resume session (no replay) | `AgentConnection.resume_session()` | P0 |
| Load session (history replay) | `AgentConnection.load_session()` | P0 |
| Close session (ref-counted) | `AgentConnection.close_session()` | P0 |
| Session modes | `AgentSessionModes` | P1 |
| Per-session model selector | `AgentModelSelector` | P1 |
| Per-session config options | `AgentSessionConfigOptions` | P1 |
| Conversation truncation | `AgentSessionTruncate` (trait object) | P2 |
| Retry last turn | `AgentSessionRetry` (trait object) | P2 |
| Set session title | `AgentSessionSetTitle` (trait object) | P2 |
| Manual compaction trigger | (User requested) | P2 |
| Protocol debug log | `AcpDebugLog` | P3 |
| Real-time session list updates | Zed watch channel / push refresh | P1 |
| ACP session workDir/project association | Zed `new_session(project, workDirs)` | P1 |

---

## 2. Frontend Scope Lock

This document now covers only frontend/app work.

### In Scope
- session list/search/new-chat UX in agent-command
- route/session state in the app
- sending `X-Session-Id` when existing backend paths already support it
- capability-gated UI that hides unsupported actions
- graceful fallback to current single-session behavior when backend support is absent

### Deferred
- all new server routes
- runtime/session persistence changes
- capability negotiation protocol changes
- replay/resume backend behavior
- ACP model/mode/config backend contracts

All deferred backend items moved to:
`docs/superpowers/specs/2026-05-12-session-management-backend-deferred.md`

## 3. Zed ACP Reference

### Capabilities Structure (from `acp.rs`)

Zed uses **nested capability objects**, not a flat struct:

```typescript
// Mirrors Zed's AgentCapabilities → SessionCapabilities nesting
interface AgentCapabilities {
  load_session: boolean
  prompt_capabilities: PromptCapabilities
  session_capabilities: SessionCapabilities
}

interface SessionCapabilities {
  list?: SessionListCapabilities      // { cursor: bool }
  close?: SessionCloseCapabilities    // { ... }
  resume?: SessionResumeCapabilities  // { ... }
}
```

### Trait Object Pattern (from `connection.rs`)

`retry`, `truncate`, `setTitle` are **separate trait objects**, not direct methods:

```typescript
// Zed pattern: connection returns trait objects for lifecycle ops
interface AgentConnection {
  // Core session lifecycle
  new_session(project, workDirs): Task<Session>
  load_session(id, ...): Task<Session>
  resume_session(id, ...): Task<Session>
  close_session(id): Task<void>

  // Capability queries
  supports_load_session(): boolean
  supports_resume_session(): boolean
  supports_close_session(): boolean
  supports_session_search(): boolean
  session_list(): AgentSessionList | null

  // Trait object accessors (return null if unsupported)
  retry(sessionId): AgentSessionRetry | null
  truncate(sessionId): AgentSessionTruncate | null
  set_title(sessionId): AgentSessionSetTitle | null
  model_selector(sessionId): AgentModelSelector | null
  session_modes(sessionId): AgentSessionModes | null
  session_config_options(sessionId): AgentSessionConfigOptions | null
}
```

---

## 4. Frontend Plan — Generalizing `sessionId: 'main'`

### 3.1 Scope of Change

Files that hardcode `'main'` and need parameterization:

| File | Lines | Change |
|------|-------|--------|
| `api/routes/agents.ts` | L328,409,417,655 | Accept `sessionId` param from route/header |
| `api/services/agents/agent-harness-service.ts` | L299,365,368,454,504,765,777,889,894,954,1023 | `sessionId` parameter on all methods |
| `lib/agents/active-turn-registry.ts` | — | Already keyed by `(agentId, sessionId)` — just needs callers to pass real IDs |
| `lib/agents/acpx-runtime.ts` | — | `getHistory()`/`getRowSnapshot()` already accept `sessionId` param |

### 4.2 Existing Backend Dependency

The ideal ACP flow still depends on backend endpoints/protocol support, but that work is **deferred**.

Frontend must therefore support two modes:

```typescript
// Preferred UX target
/agents/:agentId/chat/:sessionId

// Fallback when backend does not expose full multi-session lifecycle
/agents/:agentId/chat
```

Rules:
- send `X-Session-Id` only on paths that already honor it
- do not block the app on missing session CRUD endpoints
- hide/disable load/resume/close/title actions until capability/backend support exists
- use local UI state for session selection when persistence is unavailable

### 4.3 Frontend File Locations

```
entrypoints/app/agent-command/
├── AgentSessionList.tsx
├── AgentSessionItem.tsx
├── AgentSessionSearch.tsx
├── NewAgentSessionButton.tsx
├── useAgentSessionList.ts
├── AgentSessionListEmpty.tsx
├── AgentModeSwitch.tsx           # capability-gated, hidden if unsupported
└── useAgentSessionModes.ts       # frontend adapter only; backend support deferred
```

Backend files from earlier planning remain deferred in:
`2026-05-12-session-management-backend-deferred.md`

### 3.4 Deferred Backend Reference — Drizzle Schema

```typescript
// server/src/lib/db/schema/agent-sessions.ts
import { sqliteTable, text, integer, index } from 'drizzle-orm/sqlite-core'

export const agentSessions = sqliteTable(
  'agent_sessions',
  {
    id: text('id').primaryKey(),
    agentId: text('agent_id').notNull(),
    title: text('title'),
    cwd: text('cwd'),
    mode: text('mode', { enum: ['code', 'ask', 'agent'] }).notNull().default('agent'),
    model: text('model'),
    turnCount: integer('turn_count').notNull().default(0),
    lastMessagePreview: text('last_message_preview'),
    lastMessageAt: integer('last_message_at'),
    createdAt: integer('created_at').notNull(),
    updatedAt: integer('updated_at').notNull(),
    meta: text('meta'), // JSON
  },
  (table) => [
    index('agent_sessions_agent_id_idx').on(table.agentId, table.updatedAt),
  ],
)
```

### 3.5 Deferred Backend Reference — Ref-Counted Session Store (In-Memory)

This complements the existing `SessionStore` — it tracks **active session handles**, not agent state:

```typescript
// server/src/agent/agent-session-store.ts
// Located alongside existing session-store.ts (same module boundary)

interface ActiveSession {
  sessionId: string
  agentId: string
  refCount: number
  createdAt: number
  cwd?: string | null
}

class AgentSessionStore {
  private sessions = new Map<string, ActiveSession>()
  private pendingLoads = new Map<string, Promise<ActiveSession>>()

  async openSession(agentId: string, sessionId: string, cwd?: string): Promise<ActiveSession>
  async closeSession(sessionId: string): Promise<void>  // ref-counted
  async listSessions(agentId: string, cursor?: string, limit?: number, search?: string): Promise<SessionListResponse>
}
```

Relationship with existing `SessionStore` (from `src/agent/session-store.ts`):
- `SessionStore` = agent runtime state (AiSdkAgent, browser context, MCP servers)
- `AgentSessionStore` = session metadata + ref-counting + listing
- `AgentSessionStore` wraps `SessionStore` — when ref count hits 0, calls `SessionStore.delete(sessionId)`

### 3.6 Deferred Backend Reference — Session Capabilities (Mirrors Zed Nesting)

```typescript
interface AgentCapabilities {
  load_session: boolean
  session_capabilities: SessionCapabilities
}

interface SessionCapabilities {
  list?: { cursor: boolean }
  close?: {}
  resume?: {}
}
```

### 3.7 Deferred Backend Reference — Trait Objects for Lifecycle Ops

```typescript
// Separate from session store — these are operation handles
interface AgentSessionRetry {
  run(sessionId: string): Promise<PromptResponse>
}

interface AgentSessionTruncate {
  run(sessionId: string, messageId: string): Promise<void>
}

interface AgentSessionSetTitle {
  run(sessionId: string, title: string): Promise<void>
}
```

### 3.8 Deferred Backend Reference — Session Modes (Dynamic Discovery)

Modes are NOT hardcoded. They're discovered from the agent adapter:

```typescript
interface AgentSessionModes {
  currentMode(): SessionModeId
  allModes(): SessionMode[]
  setMode(modeId: SessionModeId): Promise<void>
}

interface SessionMode {
  id: string
  name: string
  description?: string
}
```

Available modes come from the agent's capabilities response. If the adapter doesn't expose modes, the switcher is hidden.

---

## 5. Frontend UX

### 4.1 Files

```
entrypoints/app/agent-command/
├── AgentSessionList.tsx          # Session list for selected agent
├── AgentSessionList.test.tsx
├── AgentSessionItem.tsx          # Single session row
├── AgentSessionItem.test.tsx
├── AgentSessionSearch.tsx        # Search bar
├── NewAgentSessionButton.tsx     # "New Chat" button
├── useAgentSessionList.ts        # React Query hook
├── useAgentSessionList.test.ts
├── AgentSessionListEmpty.tsx
├── AgentModeSwitch.tsx           # Dynamic mode toggle (P1)
└── useAgentSessionModes.ts       # Mode management (P1)
```

### 4.2 Flow

1. `AgentCommandHome.tsx` renders `AgentSessionList` instead of `RecentThreads`
2. User clicks session → navigate to `/agents/:agentId/chat/:sessionId`
3. `AgentCommandConversation` loads session via `load_session` or `resume_session`
4. "New Chat" → `POST /agents/:agentId/sessions` → navigate to new session
5. Existing `/:agentId/chat` route gains optional `sessionId` param (defaults to `'main'`)
6. Every ACP chat/history/cancel request must send `X-Session-Id` from the current route/session state
7. Session list view updates via push/watch if available, otherwise explicit polling fallback

---

## 6. Effort

| Layer | Files | Hours |
|-------|-------|-------|
| Generalize 'main' → multi-session | 4 files modify | 8h |
| Drizzle schema + migration | 2 new, 1 modify | 3h |
| Server: session service + routes | 3 new, 1 modify | 8h |
| Frontend: session list UI | 8 new | 12h |
| Frontend: mode + config | 3 new | 8h |
| Frontend: truncate + retry + debug | 4 new | 7h |
| **Total** | **~26** | **~46h** |

### Priority

```
P0: Frontend session list/search/new-chat + route/session state + send `X-Session-Id` where supported
P1: Mode switcher (dynamic/capability-gated) + polling refresh + degraded multi-session UX
P2: Truncation/retry/title/manual-compact UI only when backend support exists
P3: Debug/log surfaces only if backend later exposes them
```
