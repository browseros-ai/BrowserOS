# Session Management Backend — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `subagent-driven-development` or `executing-plans`. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the deferred backend work for session management — Drizzle schemas, session stores, REST routes, and ACP session lifecycle (list/create/close/resume). This unblocks the frontend-only session UX already merged in PR #9.

**Specs:**
- `docs/superpowers/specs/2026-05-12-session-management-backend-deferred.md`
- `docs/superpowers/specs/2026-05-12-acp-agent-mode-session-management.md` (sections 3.4–3.8)
- `docs/superpowers/specs/2026-05-12-assistant-workspace-session-management.md` (sections 3.1, 3.3, 6)

**Worktree:** `/home/bhd/Documents/Projects/bhd/BrowserOS-wt-session-backend`
**Branch:** `feat/session-backend` → merge to `dev`

---

## Current State

| Component | File | Status |
|-----------|------|--------|
| Agent definitions DB | `lib/db/schema/agents.ts` | ✅ |
| Runtime session store (in-memory) | `agent/session-store.ts` | ✅ `Map<string, AgentSession>` |
| ACP runtime | `lib/agents/acpx-runtime.ts` | ✅ hardcodes `sessionId: 'main'` |
| Active turn registry | `lib/agents/active-turn-registry.ts` | ✅ keyed by `(agentId, sessionId)` |
| Agent routes | `api/routes/agents.ts` | ✅ has `/:agentId/sessions/main/history` |
| Harness service | `api/services/agents/agent-harness-service.ts` | ✅ hardcodes `'main'` everywhere |

---

## File Structure

```
packages/browseros-agent/apps/server/src/
  lib/db/schema/
    agent-sessions.ts              # NEW: ACP agent session metadata table
    assistant-sessions.ts          # NEW: Assistant session metadata + workspace + tags tables
    index.ts                       # MODIFY: export new schemas

  agent/
    session-store.ts               # MODIFY: add agentId lookup method
    agent-session-store.ts         # NEW: ref-counted session metadata + listing

  lib/agents/
    active-turn-registry.ts        # MODIFY: widen sessionId from literal 'main' to string

  api/services/agents/
    agent-harness-service.ts       # MODIFY: parameterize sessionId, call AgentSessionStore

  api/routes/
    agents.ts                      # MODIFY: add session CRUD routes
    agent-sessions.ts              # NEW: session CRUD route handlers (extracted)
    assistant-sessions.ts          # NEW: assistant session routes
    server.ts                      # MODIFY: register new routes

  sessions/
    agents-md-loader.ts            # NEW: AGENTS.md reader with allowlist + cache

  api/services/
    chat-service.ts                # MODIFY: resolve workspaces from request

packages/browseros-agent/apps/server/tests/
  lib/db/schema/
    agent-sessions.test.ts         # NEW: schema + migration tests
    assistant-sessions.test.ts     # NEW: schema + migration tests

  agent/
    agent-session-store.test.ts    # NEW: ref-count, list, search, CRUD tests

  api/routes/
    agent-sessions.test.ts         # NEW: route integration tests
    assistant-sessions.test.ts     # NEW: route integration tests

  sessions/
    agents-md-loader.test.ts       # NEW: allowlist, size limit, cache tests
```

---

### Task 1: Drizzle Schemas + Migration

**Files:**
- Create: `packages/browseros-agent/apps/server/src/lib/db/schema/agent-sessions.ts`
- Create: `packages/browseros-agent/apps/server/src/lib/db/schema/assistant-sessions.ts`
- Modify: `packages/browseros-agent/apps/server/src/lib/db/schema/index.ts`
- Test: `packages/browseros-agent/apps/server/tests/lib/db/schema/agent-sessions.test.ts`
- Test: `packages/browseros-agent/apps/server/tests/lib/db/schema/assistant-sessions.test.ts`

- [ ] **Step 1: Write the failing schema tests**

```ts
// tests/lib/db/schema/agent-sessions.test.ts
import { describe, it, expect } from 'bun:test'
import { agentSessions } from '../../../../src/lib/db/schema/agent-sessions'

describe('agentSessions schema', () => {
  it('has all required columns', () => {
    const cols = Object.keys(agentSessions)
    expect(cols).toContain('id')
    expect(cols).toContain('agentId')
    expect(cols).toContain('title')
    expect(cols).toContain('cwd')
    expect(cols).toContain('mode')
    expect(cols).toContain('model')
    expect(cols).toContain('turnCount')
    expect(cols).toContain('lastMessagePreview')
    expect(cols).toContain('lastMessageAt')
    expect(cols).toContain('createdAt')
    expect(cols).toContain('updatedAt')
    expect(cols).toContain('meta')
  })
})

// tests/lib/db/schema/assistant-sessions.test.ts
import { describe, it, expect } from 'bun:test'
import { assistantSessions, sessionWorkspaces, sessionTags } from '../../../../src/lib/db/schema/assistant-sessions'

describe('assistant session schemas', () => {
  it('assistantSessions has required columns', () => {
    const cols = Object.keys(assistantSessions)
    expect(cols).toContain('id')
    expect(cols).toContain('title')
    expect(cols).toContain('mode')
    expect(cols).toContain('workspaces') // virtual FK
    expect(cols).toContain('createdAt')
    expect(cols).toContain('updatedAt')
  })

  it('sessionWorkspaces has composite structure', () => {
    const cols = Object.keys(sessionWorkspaces)
    expect(cols).toContain('sessionId')
    expect(cols).toContain('workspaceId')
    expect(cols).toContain('workspacePath')
    expect(cols).toContain('workspaceName')
  })

  it('sessionTags has tag columns', () => {
    const cols = Object.keys(sessionTags)
    expect(cols).toContain('sessionId')
    expect(cols).toContain('tag')
  })
})
```

- [ ] **Step 2: Run the tests to verify RED**

Run: `cd packages/browseros-agent && bun test apps/server/tests/lib/db/schema/agent-sessions.test.ts apps/server/tests/lib/db/schema/assistant-sessions.test.ts`
Expected: FAIL — modules don't exist.

- [ ] **Step 3: Implement agent-sessions schema**

```ts
// packages/browseros-agent/apps/server/src/lib/db/schema/agent-sessions.ts
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

- [ ] **Step 4: Implement assistant-sessions schema**

```ts
// packages/browseros-agent/apps/server/src/lib/db/schema/assistant-sessions.ts
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

- [ ] **Step 5: Update schema/index.ts exports**

```ts
export * from './agent-sessions'
export * from './assistant-sessions'
```

- [ ] **Step 6: Run tests to verify GREEN**

Run: `cd packages/browseros-agent && bun test apps/server/tests/lib/db/schema/`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add packages/browseros-agent/apps/server/src/lib/db/schema/ packages/browseros-agent/apps/server/tests/lib/db/schema/
git commit -m "feat(db): add agent_sessions + assistant_sessions + workspaces + tags schemas"
```

---

### Task 2: AgentSessionStore — Ref-Counted Session Metadata + Listing

**Files:**
- Create: `packages/browseros-agent/apps/server/src/agent/agent-session-store.ts`
- Test: `packages/browseros-agent/apps/server/tests/agent/agent-session-store.test.ts`

- [ ] **Step 1: Write the failing store tests**

```ts
// tests/agent/agent-session-store.test.ts
import { describe, it, expect, beforeEach } from 'bun:test'
import { AgentSessionStore } from '../../../src/agent/agent-session-store'

describe('AgentSessionStore', () => {
  let store: AgentSessionStore

  beforeEach(() => { store = new AgentSessionStore() })

  it('opens a new session and tracks it', async () => {
    const session = await store.openSession('agent-1', 'sess-1', '/home/user/project')
    expect(session.sessionId).toBe('sess-1')
    expect(session.agentId).toBe('agent-1')
    expect(session.refCount).toBe(1)
    expect(session.cwd).toBe('/home/user/project')
  })

  it('increments ref count on re-open', async () => {
    await store.openSession('agent-1', 'sess-1')
    const session = await store.openSession('agent-1', 'sess-1')
    expect(session.refCount).toBe(2)
  })

  it('decrements ref count on close; deletes at zero', async () => {
    await store.openSession('agent-1', 'sess-1')
    const remaining = await store.closeSession('sess-1')
    expect(remaining).toBe(0)
    const sessions = await store.listSessions('agent-1')
    expect(sessions).toHaveLength(0)
  })

  it('lists sessions for an agent ordered by updatedAt', async () => {
    await store.openSession('agent-1', 'sess-1')
    await store.openSession('agent-1', 'sess-2')
    const sessions = await store.listSessions('agent-1')
    expect(sessions).toHaveLength(2)
  })

  it('searches sessions by title', async () => {
    await store.openSession('agent-1', 'sess-1')
    await store.updateSessionMeta('sess-1', { title: 'Fix React bug' })
    const results = await store.listSessions('agent-1', { search: 'React' })
    expect(results).toHaveLength(1)
  })

  it('supports cursor-based pagination', async () => {
    for (let i = 0; i < 15; i++) {
      await store.openSession('agent-1', `sess-${i}`)
    }
    const page1 = await store.listSessions('agent-1', { limit: 10 })
    expect(page1).toHaveLength(10)
    const cursor = page1[page1.length - 1].sessionId
    const page2 = await store.listSessions('agent-1', { cursor, limit: 10 })
    expect(page2).toHaveLength(5)
  })

  it('updates session metadata', async () => {
    await store.openSession('agent-1', 'sess-1')
    await store.updateSessionMeta('sess-1', { title: 'New title', turnCount: 5 })
    const sessions = await store.listSessions('agent-1')
    expect(sessions[0].title).toBe('New title')
    expect(sessions[0].turnCount).toBe(5)
  })

  it('returns null for unknown session operations', async () => {
    const meta = await store.getSessionMeta('unknown')
    expect(meta).toBeNull()
  })
})
```

- [ ] **Step 2: Run the tests to verify RED**

Run: `cd packages/browseros-agent && bun test apps/server/tests/agent/agent-session-store.test.ts`
Expected: FAIL — module doesn't exist.

- [ ] **Step 3: Implement AgentSessionStore**

```ts
// packages/browseros-agent/apps/server/src/agent/agent-session-store.ts
import { logger } from '../lib/logger'

interface ActiveSession {
  sessionId: string
  agentId: string
  refCount: number
  createdAt: number
  updatedAt: number
  cwd?: string | null
  title?: string | null
  mode: string
  model?: string | null
  turnCount: number
  lastMessagePreview?: string | null
  lastMessageAt?: number | null
  meta?: Record<string, unknown> | null
}

export interface SessionListOptions {
  cursor?: string
  limit?: number
  search?: string
}

export interface SessionListResponse {
  sessions: ActiveSession[]
  nextCursor?: string
}

export class AgentSessionStore {
  private sessions = new Map<string, ActiveSession>()

  async openSession(agentId: string, sessionId: string, cwd?: string): Promise<ActiveSession> {
    const existing = this.sessions.get(sessionId)
    if (existing) {
      existing.refCount += 1
      existing.updatedAt = Date.now()
      logger.info('Session re-opened', { sessionId, refCount: existing.refCount })
      return existing
    }
    const session: ActiveSession = {
      sessionId,
      agentId,
      refCount: 1,
      createdAt: Date.now(),
      updatedAt: Date.now(),
      cwd: cwd ?? null,
      title: null,
      mode: 'agent',
      model: null,
      turnCount: 0,
      lastMessagePreview: null,
      lastMessageAt: null,
      meta: null,
    }
    this.sessions.set(sessionId, session)
    logger.info('Session opened', { sessionId, agentId, total: this.sessions.size })
    return session
  }

  async closeSession(sessionId: string): Promise<number> {
    const session = this.sessions.get(sessionId)
    if (!session) return 0
    session.refCount -= 1
    if (session.refCount <= 0) {
      this.sessions.delete(sessionId)
      logger.info('Session deleted (ref count 0)', { sessionId, remaining: this.sessions.size })
      return 0
    }
    logger.info('Session closed (ref > 0)', { sessionId, refCount: session.refCount })
    return session.refCount
  }

  async listSessions(agentId: string, options?: SessionListOptions): Promise<ActiveSession[]> {
    let results = [...this.sessions.values()]
      .filter(s => s.agentId === agentId)
      .sort((a, b) => b.updatedAt - a.updatedAt)

    if (options?.search) {
      const q = options.search.toLowerCase()
      results = results.filter(s =>
        s.title?.toLowerCase().includes(q) ||
        s.lastMessagePreview?.toLowerCase().includes(q)
      )
    }

    if (options?.cursor) {
      const idx = results.findIndex(s => s.sessionId === options.cursor)
      if (idx >= 0) results = results.slice(idx + 1)
    }

    if (options?.limit) {
      results = results.slice(0, options.limit)
    }

    return results
  }

  async getSessionMeta(sessionId: string): Promise<ActiveSession | null> {
    return this.sessions.get(sessionId) ?? null
  }

  async updateSessionMeta(sessionId: string, updates: Partial<Pick<ActiveSession, 'title' | 'turnCount' | 'lastMessagePreview' | 'lastMessageAt' | 'mode' | 'model' | 'meta'>>): Promise<ActiveSession | null> {
    const session = this.sessions.get(sessionId)
    if (!session) return null
    Object.assign(session, updates, { updatedAt: Date.now() })
    return session
  }
}
```

- [ ] **Step 4: Run the tests to verify GREEN**

Run: `cd packages/browseros-agent && bun test apps/server/tests/agent/agent-session-store.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add packages/browseros-agent/apps/server/src/agent/agent-session-store.ts packages/browseros-agent/apps/server/tests/agent/agent-session-store.test.ts
git commit -m "feat(agent): add AgentSessionStore with ref-counting and listing"
```

---

### Task 3: Widen sessionId from `'main'` literal to `string`

**Files:**
- Modify: `packages/browseros-agent/apps/server/src/lib/agents/active-turn-registry.ts`
- Modify: `packages/browseros-agent/apps/server/src/api/services/agents/agent-harness-service.ts`
- Modify: `packages/browseros-agent/apps/server/src/lib/agents/acpx-runtime.ts`
- Test: existing tests must still pass

- [ ] **Step 1: Write failing type-change tests**

```ts
// In the existing agent-harness-service test, add:
it('accepts arbitrary sessionId strings beyond "main"', async () => {
  // Verify the service methods accept sessionId: string
  const turn = service.turnRegistry.register('test-agent', 'custom-session-123', {
    prompt: 'hello',
    abortController: new AbortController(),
  })
  const active = service.turnRegistry.getActiveFor('test-agent', 'custom-session-123')
  expect(active).toBeDefined()
  expect(active?.sessionId).toBe('custom-session-123')
})
```

- [ ] **Step 2: Run tests to verify RED**

Run: `cd packages/browseros-agent && bun test apps/server/tests/lib/agents/ apps/server/tests/api/services/agents/`
Expected: FAIL — type literal `'main'` prevents string.

- [ ] **Step 3: Widen types in active-turn-registry.ts**

Change `sessionId: 'main'` to `sessionId: string` in:
- `ActiveTurnInfo` interface
- `ActiveTurn` interface
- All method signatures

- [ ] **Step 4: Widen types in agent-harness-service.ts**

Change all `sessionId: 'main'` occurrences to accept `string` parameter, defaulting to `'main'`.

- [ ] **Step 5: Widen types in acpx-runtime.ts**

Change `sessionId: 'main'` to `sessionId: string` in `getHistory`, `getRowSnapshot`, `send`, and internal methods.

- [ ] **Step 6: Run all existing tests to verify GREEN**

Run: `cd packages/browseros-agent && bun test apps/server/tests/`
Expected: All pass — backward compatible (default remains `'main'`).

- [ ] **Step 7: Commit**

```bash
git add packages/browseros-agent/apps/server/src/lib/agents/active-turn-registry.ts packages/browseros-agent/apps/server/src/api/services/agents/agent-harness-service.ts packages/browseros-agent/apps/server/src/lib/agents/acpx-runtime.ts
git commit -m "refactor: widen sessionId from literal 'main' to string"
```

---

### Task 4: Session CRUD REST Routes

**Files:**
- Create: `packages/browseros-agent/apps/server/src/api/routes/agent-sessions.ts`
- Create: `packages/browseros-agent/apps/server/src/api/routes/assistant-sessions.ts`
- Modify: `packages/browseros-agent/apps/server/src/api/routes/agents.ts` (parameterize sessionId in existing routes)
- Modify: `packages/browseros-agent/apps/server/src/api/server.ts`
- Test: `packages/browseros-agent/apps/server/tests/api/routes/agent-sessions.test.ts`
- Test: `packages/browseros-agent/apps/server/tests/api/routes/assistant-sessions.test.ts`

- [ ] **Step 1: Write the failing route tests**

```ts
// tests/api/routes/agent-sessions.test.ts
import { describe, it, expect, beforeEach } from 'bun:test'
import { createAgentSessionRoutes } from '../../../../src/api/routes/agent-sessions'
import { AgentSessionStore } from '../../../../src/agent/agent-session-store'
import type { Hono } from 'hono'

describe('Agent Session Routes', () => {
  let app: Hono
  let store: AgentSessionStore

  beforeEach(() => {
    store = new AgentSessionStore()
    app = createAgentSessionRoutes({ store })
  })

  it('POST /:agentId/sessions creates a new session', async () => {
    const res = await app.request('/agent-1/sessions', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ cwd: '/home/user/project' }),
    })
    expect(res.status).toBe(201)
    const data = await res.json()
    expect(data.session).toBeDefined()
    expect(data.session.agentId).toBe('agent-1')
  })

  it('GET /:agentId/sessions lists sessions', async () => {
    await store.openSession('agent-1', 's1')
    await store.openSession('agent-1', 's2')
    const res = await app.request('/agent-1/sessions')
    expect(res.status).toBe(200)
    const data = await res.json()
    expect(data.sessions).toHaveLength(2)
  })

  it('GET /:agentId/sessions?search= filters by title', async () => {
    await store.openSession('agent-1', 's1')
    await store.updateSessionMeta('s1', { title: 'Fix React bug' })
    await store.openSession('agent-1', 's2')
    await store.updateSessionMeta('s2', { title: 'Add dark mode' })
    const res = await app.request('/agent-1/sessions?search=React')
    expect(res.status).toBe(200)
    const data = await res.json()
    expect(data.sessions).toHaveLength(1)
  })

  it('DELETE /:agentId/sessions/:sessionId closes a session', async () => {
    await store.openSession('agent-1', 's1')
    const res = await app.request('/agent-1/sessions/s1', { method: 'DELETE' })
    expect(res.status).toBe(200)
    const data = await res.json()
    expect(data.ok).toBe(true)
  })

  it('PATCH /:agentId/sessions/:sessionId updates metadata', async () => {
    await store.openSession('agent-1', 's1')
    const res = await app.request('/agent-1/sessions/s1', {
      method: 'PATCH',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ title: 'New title' }),
    })
    expect(res.status).toBe(200)
    const data = await res.json()
    expect(data.session.title).toBe('New title')
  })
})
```

- [ ] **Step 2: Run the tests to verify RED**

Run: `cd packages/browseros-agent && bun test apps/server/tests/api/routes/agent-sessions.test.ts`
Expected: FAIL — module doesn't exist.

- [ ] **Step 3: Implement agent session routes**

```ts
// packages/browseros-agent/apps/server/src/api/routes/agent-sessions.ts
import { Hono } from 'hono'
import { randomUUID } from 'node:crypto'
import type { AgentSessionStore } from '../../agent/agent-session-store'

export function createAgentSessionRoutes(deps: { store: AgentSessionStore }) {
  const { store } = deps
  return new Hono()
    .get('/:agentId/sessions', async (c) => {
      const agentId = c.req.param('agentId')
      const search = c.req.query('search')
      const cursor = c.req.query('cursor')
      const limit = c.req.query('limit') ? Number(c.req.query('limit')) : undefined
      const sessions = await store.listSessions(agentId, { search, cursor, limit })
      return c.json({ sessions })
    })
    .post('/:agentId/sessions', async (c) => {
      const agentId = c.req.param('agentId')
      const body = await c.req.json().catch(() => ({}))
      const sessionId = randomUUID()
      const session = await store.openSession(agentId, sessionId, body.cwd)
      return c.json({ session }, 201)
    })
    .get('/:agentId/sessions/:sessionId', async (c) => {
      const session = await store.getSessionMeta(c.req.param('sessionId'))
      if (!session) return c.json({ error: 'Session not found' }, 404)
      return c.json({ session })
    })
    .patch('/:agentId/sessions/:sessionId', async (c) => {
      const body = await c.req.json()
      const session = await store.updateSessionMeta(c.req.param('sessionId'), body)
      if (!session) return c.json({ error: 'Session not found' }, 404)
      return c.json({ session })
    })
    .delete('/:agentId/sessions/:sessionId', async (c) => {
      await store.closeSession(c.req.param('sessionId'))
      return c.json({ ok: true })
    })
}
```

- [ ] **Step 4: Implement assistant session routes (similar pattern)**

```ts
// packages/browseros-agent/apps/server/src/api/routes/assistant-sessions.ts
// Similar CRUD for assistant sessions, with workspace/tag support
```

- [ ] **Step 5: Parameterize existing agent route sessionId**

In `agents.ts`:
- Change `/:agentId/sessions/main/history` to `/:agentId/sessions/:sessionId/history`
- Default `sessionId` to `'main'` when not provided
- Pass real sessionId to harness service calls

- [ ] **Step 6: Register routes in server.ts**

```ts
import { createAgentSessionRoutes } from './routes/agent-sessions'
import { createAssistantSessionRoutes } from './routes/assistant-sessions'

// Inside route setup:
.route('/agent-sessions', createAgentSessionRoutes({ store }))
.route('/assistant-sessions', createAssistantSessionRoutes({ store }))
```

- [ ] **Step 7: Run tests to verify GREEN**

Run: `cd packages/browseros-agent && bun test apps/server/tests/api/routes/`
Expected: All pass.

- [ ] **Step 8: Commit**

```bash
git add packages/browseros-agent/apps/server/src/api/routes/agent-sessions.ts packages/browseros-agent/apps/server/src/api/routes/assistant-sessions.ts packages/browseros-agent/apps/server/src/api/routes/agents.ts packages/browseros-agent/apps/server/src/api/server.ts packages/browseros-agent/apps/server/tests/api/routes/
git commit -m "feat(routes): add session CRUD routes for ACP agents + assistant"
```

---

### Task 5: AGENTS.md Loader with Allowlist + Cache

**Files:**
- Create: `packages/browseros-agent/apps/server/src/sessions/agents-md-loader.ts`
- Test: `packages/browseros-agent/apps/server/tests/sessions/agents-md-loader.test.ts`

- [ ] **Step 1: Write the failing loader tests**

```ts
import { describe, it, expect, beforeEach, afterEach } from 'bun:test'
import { mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { AgentsMdLoader } from '../../../src/sessions/agents-md-loader'

describe('AgentsMdLoader', () => {
  let testDir: string

  beforeEach(async () => {
    testDir = await mkdtemp(join(tmpdir(), 'agents-md-'))
  })

  afterEach(async () => {
    await rm(testDir, { recursive: true })
  })

  it('loads AGENTS.md from allowed workspace path', async () => {
    await writeFile(join(testDir, 'AGENTS.md'), '# Test Instructions\nUse TypeScript')
    const loader = new AgentsMdLoader([testDir])
    const result = await loader.load(testDir)
    expect(result).toBeDefined()
    expect(result!.content).toContain('Use TypeScript')
  })

  it('returns null for disallowed path', async () => {
    await writeFile(join(testDir, 'AGENTS.md'), 'content')
    const loader = new AgentsMdLoader(['/other/path'])
    const result = await loader.load(testDir)
    expect(result).toBeNull()
  })

  it('returns null when AGENTS.md does not exist', async () => {
    const loader = new AgentsMdLoader([testDir])
    const result = await loader.load(testDir)
    expect(result).toBeNull()
  })

  it('rejects files exceeding 100KB size limit', async () => {
    const big = 'x'.repeat(101 * 1024)
    await writeFile(join(testDir, 'AGENTS.md'), big)
    const loader = new AgentsMdLoader([testDir])
    const result = await loader.load(testDir)
    expect(result).toBeNull()
  })

  it('caches and detects changes via mtime', async () => {
    await writeFile(join(testDir, 'AGENTS.md'), 'v1')
    const loader = new AgentsMdLoader([testDir])
    const r1 = await loader.load(testDir)
    expect(r1!.content).toBe('v1')

    await writeFile(join(testDir, 'AGENTS.md'), 'v2')
    const r2 = await loader.load(testDir)
    expect(r2!.content).toBe('v2')
  })

  it('loads multiple workspace AGENTS.md files', async () => {
    const dir1 = join(testDir, 'frontend')
    const dir2 = join(testDir, 'backend')
    await mkdir(dir1, { recursive: true })
    await mkdir(dir2, { recursive: true })
    await writeFile(join(dir1, 'AGENTS.md'), 'Frontend rules')
    await writeFile(join(dir2, 'AGENTS.md'), 'Backend rules')

    const loader = new AgentsMdLoader([dir1, dir2])
    const results = await loader.loadMultiple([dir1, dir2])
    expect(results).toHaveLength(2)
  })
})
```

- [ ] **Step 2: Run tests to verify RED**

Run: `cd packages/browseros-agent && bun test apps/server/tests/sessions/agents-md-loader.test.ts`
Expected: FAIL — module doesn't exist.

- [ ] **Step 3: Implement AgentsMdLoader**

```ts
// packages/browseros-agent/apps/server/src/sessions/agents-md-loader.ts
import { readFile, stat } from 'node:fs/promises'
import { join, resolve, sep } from 'node:path'

const MAX_AGENTS_MD_SIZE = 100 * 1024 // 100KB

interface AgentsMdResult {
  path: string
  content: string
  lastModified: number
}

interface CacheEntry {
  result: AgentsMdResult
  lastModified: number
}

export class AgentsMdLoader {
  private allowlist: Set<string>
  private cache = new Map<string, CacheEntry>()

  constructor(registeredPaths: string[]) {
    this.allowlist = new Set(registeredPaths.map(p => resolve(p)))
  }

  async load(workspacePath: string): Promise<AgentsMdResult | null> {
    const resolved = resolve(workspacePath)
    if (!this.allowlist.has(resolved)) return null

    const filePath = join(resolved, 'AGENTS.md')
    if (!filePath.startsWith(resolved + sep)) return null

    const fileStat = await stat(filePath).catch(() => null)
    if (!fileStat || fileStat.size > MAX_AGENTS_MD_SIZE) return null

    // Cache check
    const cached = this.cache.get(resolved)
    if (cached && cached.lastModified === fileStat.mtimeMs) {
      return cached.result
    }

    const content = await readFile(filePath, 'utf-8')
    const result: AgentsMdResult = {
      path: filePath,
      content,
      lastModified: fileStat.mtimeMs,
    }
    this.cache.set(resolved, { result, lastModified: fileStat.mtimeMs })
    return result
  }

  async loadMultiple(paths: string[]): Promise<AgentsMdResult[]> {
    const results: AgentsMdResult[] = []
    for (const p of paths) {
      const r = await this.load(p)
      if (r) results.push(r)
    }
    return results
  }
}
```

- [ ] **Step 4: Run tests to verify GREEN**

Run: `cd packages/browseros-agent && bun test apps/server/tests/sessions/agents-md-loader.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add packages/browseros-agent/apps/server/src/sessions/ packages/browseros-agent/apps/server/tests/sessions/
git commit -m "feat(sessions): add AGENTS.md loader with allowlist + cache"
```

---

### Task 6: Parameterize Harness Service for Multi-Session

**Files:**
- Modify: `packages/browseros-agent/apps/server/src/api/services/agents/agent-harness-service.ts`
- Test: existing harness tests must pass

- [ ] **Step 1: Add multi-session test**

```ts
it('uses sessionId from route parameter instead of hardcoded main', async () => {
  // Open a session via the store
  const session = await harnessService.sessionStore.openSession('test-agent', 'custom-sess')
  // Verify harness methods accept the sessionId
  const history = await harnessService.getHistory('test-agent', 'custom-sess')
  // Should not throw
  expect(history).toBeDefined()
})
```

- [ ] **Step 2: Modify harness service**

Wire `AgentSessionStore` into `AgentHarnessService`. Change all methods that hardcode `'main'` to accept `sessionId` parameter. Update the existing `getHistory` and `send` methods.

- [ ] **Step 3: Run all tests to verify GREEN**

Run: `cd packages/browseros-agent && bun test apps/server/tests/`
Expected: All pass.

- [ ] **Step 4: Commit**

```bash
git add packages/browseros-agent/apps/server/src/api/services/agents/agent-harness-service.ts
git commit -m "refactor(harness): parameterize sessionId for multi-session support"
```

---

### Task 7: Integration Verification

- [ ] **Step 1: Run all server tests**

Run: `cd packages/browseros-agent && bun test apps/server/tests/`
Expected: All pass.

- [ ] **Step 2: Verify route registration**

Run: `cd packages/browseros-agent && bun test apps/server/tests/api/routes/`
Expected: All route tests pass including new session routes.

- [ ] **Step 3: Run `gitnexus_detect_changes`**

Run: `gitnexus_detect_changes` — verify change scope matches plan.

- [ ] **Step 4: Final commit if fixes needed**

```bash
git add -A
git commit -m "fix: address integration issues from session management backend"
```

---

## Self-Review Checklist

- [ ] Schema coverage: agent_sessions + assistant_sessions + session_workspaces + session_tags
- [ ] Store coverage: AgentSessionStore with open/close/list/search/pagination/update
- [ ] Route coverage: CRUD for agent sessions + assistant sessions + parameterized history
- [ ] Security: AGENTS.md allowlist + size limit + path traversal prevention
- [ ] Backward compat: all `sessionId` defaults to `'main'`, existing routes unchanged
- [ ] No TODO/TBD placeholders in committed code
