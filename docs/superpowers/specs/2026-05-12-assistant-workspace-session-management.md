# Normal Assistant — Workspace-Tagged Session Management Spec

Date: 2026-05-12
Status: DRAFT v3 (frontend-only refocus)
Mode: **Assistant Mode** (sidepanel, built-in BrowserOS AI)
Scope: **Frontend-only** workspace-tagged session UX

**Scope lock**: Backend/server/persistence changes are deferred.
See: `docs/superpowers/specs/2026-05-12-session-management-backend-deferred.md`

---

## 1. Current State — Assistant Mode (Sidepanel)

### What Exists

| Feature | File | Status |
|---------|------|--------|
| Chat send/receive (SSE) | `useChatSession.ts` → `ChatService` | ✅ |
| Conversation storage (local) | `conversationStorage.ts` | ✅ |
| Conversation storage (remote) | `ChatHistory.tsx` → GraphQL | ✅ |
| Workspace selector (global) | `ChatFooter.tsx` → `WorkspaceSelector` | ✅ |
| Workspace → system prompt | `prompt.ts` `getWorkspace()` | ✅ |
| **Workspace switch mid-conversation** | `chat-service.ts` L138-163 | ✅ Sends dynamic system message |
| Auto-compaction | `server/src/agent/compaction.ts` | ✅ |
| Chat mode toggle (chat/agent) | `ChatModeToggle.tsx` | ✅ |
| Date-grouped history | `ChatHistory.tsx` → `groupConversations()` | ✅ |
| Agent definitions DB (Drizzle) | `lib/db/schema/agents.ts` | ✅ |

### Existing Workspace Switch Behavior (chat-service.ts L138-163)

When user changes workspace mid-conversation, the server:
1. Detects `session.workingDir !== request.userWorkingDir`
2. Logs the change
3. Injects a dynamic system message: "The user switched workspace during this conversation. Filesystem tools now use the new working directory: ..."
4. Rebuilds the session with new tools

**This behavior continues** but evolves: the dynamic message is replaced by AGENTS.md injection when a workspace has one.

### What's Missing

| Feature | Description | Priority |
|---------|-------------|----------|
| **Workspace per session** (not global) | Workspace is currently global, not per-conversation | P0 |
| **Workspace bubbles** | Visual badge showing session's workspace(s) | P0 |
| **Grouped history by workspace** | Sessions grouped by workspace, then date | P0 |
| **Multi-workspace sessions** | Session can accumulate multiple workspaces | P0 |
| **Multi-group display** | Session with N workspaces → appears in N groups | P0 |
| **AGENTS.md auto-loading** | Read workspace AGENTS.md into system prompt | P0 |
| **Multi-workspace AGENTS.md** | Merge multiple AGENTS.md files | P0 |
| **Session search** | Search conversations by title/content | P1 |
| **Explicit new chat** | Button to start fresh session | P1 |
| **Resume previous session** | Restore/open an earlier conversation without refresh hacks | P1 |
| **Pagination / load more** | Browse older sessions beyond first page | P1 |
| **Manual compaction** | User-triggered "Summarize & compact" | P2 |
| **Session title edit** | Inline rename conversation | P2 |
| **Session tags** | User-defined tags for grouping | P2 |
| **Collapsible workspace groups** | Expand/collapse group sections in history | P2 |

---

## 2. Frontend Scope Lock

This document now covers only frontend/app work.

### In Scope
- workspace bubbles and grouped history in sidepanel
- local/session UI state and request-shape usage against existing APIs
- explicit new chat / resume / pagination UX in frontend
- graceful degradation when server/GraphQL/session APIs do not expose workspace metadata

### Deferred
- new persistence/schema work
- migration jobs
- GraphQL/server response changes
- AGENTS.md persistence/config changes
- server-side search/compact behavior beyond already-existing endpoints

All deferred backend items moved to:
`docs/superpowers/specs/2026-05-12-session-management-backend-deferred.md`

## 3. Frontend Data Shape

This section defines the **frontend data contract** the UI wants to consume. Any server/schema work implied below is deferred to `2026-05-12-session-management-backend-deferred.md`.

### 3.1 Deferred Backend Reference — Drizzle Schema

```typescript
// server/src/lib/db/schema/assistant-sessions.ts
import { sqliteTable, text, integer, index, primaryKey } from 'drizzle-orm/sqlite-core'

export const assistantSessions = sqliteTable(
  'assistant_sessions',
  {
    id: text('id').primaryKey(),
    title: text('title'),
    mode: text('mode', { enum: ['chat', 'agent'] }).notNull().default('chat'),
    model: text('model'),
    messageCount: integer('message_count').notNull().default(0),
    lastMessagePreview: text('last_message_preview'),
    lastMessageAt: integer('last_message_at'),
    createdAt: integer('created_at').notNull(),
    updatedAt: integer('updated_at').notNull(),
    meta: text('meta'), // JSON
  },
  (table) => [
    index('assistant_sessions_updated_idx').on(table.updatedAt),
  ],
)

export const sessionWorkspaces = sqliteTable(
  'session_workspaces',
  {
    sessionId: text('session_id').notNull(),
    workspaceId: text('workspace_id').notNull(),
    workspacePath: text('workspace_path').notNull(),
    workspaceName: text('workspace_name').notNull(),
  },
  (table) => [
    // Composite PK prevents duplicate (sessionId, workspaceId) pairs
    // Required for onConflictDoNothing() in upsert logic
    primaryKey({ columns: [table.sessionId, table.workspaceId] }),
    index('session_workspaces_session_idx').on(table.sessionId),
    index('session_workspaces_path_idx').on(table.workspacePath),
  ],
)

export const sessionTags = sqliteTable(
  'session_tags',
  {
    sessionId: text('session_id').notNull(),
    tag: text('tag').notNull(),
  },
  (table) => [
    index('session_tags_tag_idx').on(table.tag),
  ],
)
```

### 3.2 Frontend TypeScript Types

```typescript
interface AssistantSession {
  id: string
  title: string | null
  mode: 'chat' | 'agent'
  workspaces: SessionWorkspace[]
  model: string | null
  createdAt: string
  updatedAt: string
  lastMessagePreview: string | null
  lastMessageAt: number | null
  messageCount: number
  tags: string[]
  meta: Record<string, unknown> | null
}

interface SessionWorkspace {
  sessionId: string
  workspaceId: string
  workspacePath: string
  workspaceName: string
}
```

### 3.3 Deferred Backend Reference — Backward Compatibility Migration

Existing `local:conversations` data migrates on first load:

1. Read all from `conversationStorage` (WXT storage)
2. For each `Conversation`, create `AssistantSession` (workspace = null, tags = [])
3. POST to `/assistant/sessions` (server creates metadata record)
4. Messages stay in their existing storage (WXT for local, GraphQL for remote)
5. The migration only moves **metadata** (title, dates, workspace), not message content
6. Keep `local:conversations` as-is for 30 days as backup
7. If server isn't reachable (offline), continue using local storage — migration defers
8. Migration status must be explicit and idempotent so existing users are not silently dropped from the new session metadata UI

---

## 4. Existing Request Contract Usage

### Current Schema (api/types.ts)

```typescript
userWorkingDir: z.string().min(1).optional(),  // single optional string
```

### Target Schema (backward-compatible)

```typescript
// Accept both old and new formats
userWorkingDir: z.string().min(1).optional(),                    // legacy: single dir
userWorkspaces: z.array(z.object({                                // new: multi-workspace
  id: z.string(),
  path: z.string(),
  name: z.string(),
})).optional(),
```

Server logic:
```typescript
// Resolve workspaces from either format
const workspaces = request.userWorkspaces ?? (
  request.userWorkingDir
    ? [{ id: hashPath(request.userWorkingDir), path: request.userWorkingDir, name: basename(request.userWorkingDir) }]
    : []
)
```

Client sends `userWorkspaces` when available, falls back to `userWorkingDir` for compatibility.

---

## 5. Workspace-Tagged Sessions
### 4.1 Session ↔ Workspace Association

**Current**: `selectedWorkspaceStorage` is global — changes affect ALL conversations.

**Target**:
- When user sends a message, current workspace is **saved on that session**
- If user changes workspace mid-conversation, session **accumulates** a second workspace
- Workspace association is additive (never removed unless user explicitly clears)
- Each workspace addition triggers AGENTS.md load for that workspace

### 4.2 Flow

```
User opens sidepanel
  → new session created (workspaces: [])
  → user selects workspace "frontend"
  → user sends message
  → client sends: { userWorkspaces: [{ id: "abc", path: "~/frontend", name: "frontend" }] }
  → server: upsert session_workspaces row
  → server: load ~/frontend/AGENTS.md → inject into system prompt

User changes workspace to "backend" mid-session
  → client sends: { userWorkspaces: [
      { id: "abc", path: "~/frontend", name: "frontend" },
      { id: "def", path: "~/backend", name: "backend" },
    ]}
  → server: add session_workspaces row for backend
  → server: load ~/backend/AGENTS.md → merge with frontend AGENTS.md
  → Existing workspace switch message STILL fires (evolution: enhanced with AGENTS.md)
  → Session now appears in BOTH workspace groups in history
```

### 4.3 Workspace Bubble Component

**File**: `packages/browseros-agent/apps/agent/components/elements/`

```
elements/
├── WorkspaceBubble.tsx       # Single workspace badge
├── WorkspaceBubble.test.tsx
├── WorkspaceBubbleGroup.tsx  # Multiple badges inline
└── WorkspaceBubbleGroup.test.tsx
```

```typescript
interface WorkspaceBubbleProps {
  name: string
  path: string
  color?: string     // auto-assigned hash of workspace ID
  size?: 'sm' | 'md'
  onClick?: () => void
}
```

**Rendering**:

```
Single:     🔵 frontend
Multiple:   🔵 frontend  🟢 backend
None:       🌐
```

---

## 6. Session Grouping

### 5.1 Grouping Modes

```typescript
type GroupingMode = 'workspace' | 'tag' | 'date'

interface SessionGroup {
  key: string
  label: string
  icon: ReactNode
  color?: string
  sessions: AssistantSession[]
}
```

| Mode | Group Key | Label | Multi-membership |
|------|-----------|-------|-----------------|
| `workspace` | workspace ID | workspace name | Session with N workspaces → N groups |
| `tag` | tag string | `tag: value` | Session with N tags → N groups |
| `date` | today/week/month/older | "Today" etc. | Exactly 1 group |

**Default**: `workspace` when ≥1 workspace exists. Falls back to `date`.

### 5.2 Workspace Group Display

```
┌─────────────────────────────────────────────────┐
│ 📁 frontend (~/code/frontend)              [4] ▾│
│ ─────────────────────────────────────────────── │
│ 🔵  "Fix React state bug"            2h ago    │
│ 🔵  "Add dark mode toggle"           5h ago    │
│ 🔵🟢 "Fix CORS issue"                1h ago    │
│ 🔵  "Update TypeScript config"       1d ago    │
├─────────────────────────────────────────────────┤
│ 📁 backend (~/code/backend)                [2] ▾│
│ ─────────────────────────────────────────────── │
│ 🟢🔵 "Fix CORS issue"                1h ago    │ ← SAME session
│ 🟢  "Add rate limiting"              3h ago    │
├─────────────────────────────────────────────────┤
│ 🌐 No workspace                           [1] ▾│
│      "Summarize this page"           30m ago   │
└─────────────────────────────────────────────────┘
```

### 5.3 Files

```
entrypoints/sidepanel/history/
├── ChatHistory.tsx                 # MODIFY: add grouping mode switcher
├── components/
│   ├── ConversationList.tsx        # MODIFY: accept grouped data
│   ├── WorkspaceGroup.tsx          # NEW: group header + collapse
│   ├── ConversationItem.tsx        # MODIFY: add workspace bubbles
│   ├── ConversationGroup.tsx       # MODIFY: support workspace groups
│   ├── SearchBar.tsx               # NEW: search input
│   └── GroupingToggle.tsx          # NEW: workspace / tag / date switcher
├── useSessionGrouping.ts           # NEW: grouping logic
└── useSessionGrouping.test.ts
```

**Non-negotiable UX behaviors**:
- `SearchBar` must exist in the rendered history UI, not only in the file plan
- pagination/load-more must be rendered in the conversation list if the backend reports `nextCursor`
- workspace groups must be collapsible (`WorkspaceGroup.tsx` or equivalent), not flat-only

---

## 7. Existing Backend Dependency Notes

AGENTS.md loading and persistence behavior depend on backend support and are therefore deferred as implementation work. Frontend must treat them as existing-platform capabilities, not modify the server.

Frontend rules:
- render workspace UX without assuming AGENTS.md metadata is returned from the server
- if backend does not expose per-session config, do not fake persistence
- if compact endpoint is stubbed, hide or disable compact action
- if remote conversations lack workspace metadata, fall back to local/date grouping

## 8. AGENTS.md / Backend Dependency Notes

Everything in this section is dependency/fallback planning for the frontend. Server changes remain deferred.

### 6.1 Concept

When a session has a workspace, the server reads `<workspace>/AGENTS.md` and injects it into the system prompt. This is equivalent to `.cursorrules` / `CLAUDE.md` in other tools.

### 6.2 Security Model

BrowserOS runs as a local process (AppImage + local server). Workspace paths come from the BrowserOS adapter's `choosePath()` — the user explicitly selected these directories. Security constraints:

1. **Path allowlist**: Only read AGENTS.md from paths registered as workspaces in `session_workspaces` table
2. **No path traversal**: Validate resolved path starts with a registered workspace path
3. **Size limit**: Max 100KB per AGENTS.md file
4. **Server-side only**: Files are read by the local server process, not the browser extension

```typescript
class AgentsMdLoader {
  // Only allow reading from registered workspace paths
  private workspaceAllowlist: Set<string>

  constructor(registeredPaths: string[]) {
    this.workspaceAllowlist = new Set(registeredPaths)
  }

  async load(workspacePath: string): Promise<AgentsMdResult | null> {
    // Security: only read from allowed paths
    const resolved = path.resolve(workspacePath)
    if (!this.workspaceAllowlist.has(resolved)) return null

    const filePath = path.join(resolved, 'AGENTS.md')
    // Verify still within workspace (no traversal)
    if (!filePath.startsWith(resolved + path.sep)) return null

    const stat = await fs.stat(filePath).catch(() => null)
    if (!stat || stat.size > MAX_AGENTS_MD_SIZE) return null
    // ... cache + read logic
  }
}
```

### 6.3 Files

```
server/src/
├── sessions/
│   ├── agents-md-loader.ts         # Read + cache AGENTS.md (with allowlist)
│   └── agents-md-loader.test.ts
└── agent/
    └── prompt.ts                   # MODIFY: inject AGENTS.md content
```

### 6.4 System Prompt Injection

**Interface modification required** — `BuildSystemPromptOptions` (prompt.ts L743-755) needs a new field:

```typescript
interface BuildSystemPromptOptions {
  // ... existing fields ...
  workspaceDir?: string
  // NEW: loaded AGENTS.md content for injection
  workspaceAgentsMd?: Array<{ path: string; content: string; lastModified: number }>
}
```

In `prompt.ts`, after the existing `<workspace>` section:

```typescript
// EXISTING: <workspace> section with filesystem tools + dynamic switch message
// EVOLUTION: when AGENTS.md exists, the dynamic switch message is replaced

if (options?.workspaceAgentsMd?.length) {
  section += '\n\n<workspace_instructions>'
  for (const doc of options.workspaceAgentsMd) {
    section += `\n### ${doc.path}\n\n${doc.content}\n`
  }
  section += '</workspace_instructions>'
}
```

The existing workspace switch system message (chat-service.ts L138-163) continues to fire for cases where AGENTS.md doesn't exist. When AGENTS.md IS present, the switch message is **suppressed** in favor of the richer AGENTS.md content.

### 6.5 Loading Triggers

| Trigger | Action |
|---------|--------|
| Session created with workspace | Load AGENTS.md → inject into system prompt |
| Workspace added to session | Load new AGENTS.md → rebuild prompt |
| Workspace removed from session | Remove that AGENTS.md → rebuild prompt and remove session↔workspace association |
| File changed on disk | Detect on next prompt cycle (stat mtime check) |

### 6.6 Multi-Workspace Merging

```
<workspace_instructions>

### /home/user/code/frontend/AGENTS.md

(React project instructions: use TypeScript, prefer function components...)

### /home/user/code/backend/AGENTS.md

(Go API project instructions: use standard library handlers...)

</workspace_instructions>
```

### 6.7 Config Override

```typescript
// Per-session config
interface SessionConfig {
  autoLoadAgentsMd: boolean  // default: true
}
```

This override must be persisted per session and exposed in both the session API and UI; otherwise AGENTS.md loading is effectively hardcoded-on.

---

## 9. Existing Endpoint Assumptions

### 9.1 Existing/Expected Routes

Frontend may use these routes only if they already exist in the platform build it is shipped against:

| Method | Path | Purpose |
|--------|------|---------|
| `GET` | `/assistant/sessions` | List (cursor, filter by workspace/tag/search) |
| `POST` | `/assistant/sessions` | Create |
| `GET` | `/assistant/sessions/:id` | Get info |
| `PATCH` | `/assistant/sessions/:id` | Update (title, workspaces, tags, config) |
| `DELETE` | `/assistant/sessions/:id` | Delete |
| `POST` | `/assistant/sessions/:id/compact` | Trigger compaction |

If these are absent or partial, frontend must degrade gracefully instead of requiring backend changes in this phase.

### 9.2 Deferred Backend Reference — Workspace Update in ChatService

```typescript
// In ChatService.processMessage():
const workspaces = resolveWorkspaces(request) // from userWorkspaces or userWorkingDir

// Upsert workspace associations
for (const ws of workspaces) {
  await db.insert(sessionWorkspaces)
    .values({ sessionId: request.conversationId, ...ws })
    .onConflictDoNothing()  // idempotent
}

// Load AGENTS.md for all session workspaces (with allowlist)
const loader = new AgentsMdLoader(workspaces.map(w => w.path))
const agentsMd = await loader.loadMultiple(workspaces.map(w => w.path))

// Build system prompt
const systemPrompt = buildSystemPrompt({
  ...existingOptions,
  workspaceAgentsMd: agentsMd,
})
```

---

## 10. Effort

| Layer | Files | Hours |
|-------|-------|-------|
| Drizzle schema + migration | 2 new, 1 modify | 4h |
| Server: session routes + AGENTS.md loader | 4 new | 8h |
| Server: ChatService + ChatRequestSchema | 2 modify | 4h |
| Frontend: workspace bubbles | 4 new | 4h |
| Frontend: grouped history | 6 modify/new | 8h |
| Frontend: session-workspace binding | 3 modify | 6h |
| Frontend: search + new chat | 3 new | 4h |
| Frontend: manual compact + title + tags | 3 new | 4h |
| **Total** | **~28** | **~42h** |

### Priority

```
P0: Workspace per session + bubbles + grouping + explicit frontend new-chat/resume UX
P1: Session search + rendered pagination/load-more + graceful remote/local fallback when workspace metadata is missing
P2: Manual compact/title/tags/group collapse/config UX only when backed by existing platform support
```
