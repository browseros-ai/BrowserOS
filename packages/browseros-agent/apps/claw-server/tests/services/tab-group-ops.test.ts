/**
 * Stubs the `@browseros/browser-mcp` browser-tool framework so we can drive
 * the orchestrator with synthetic results and assert on the dispatch
 * shape. Bun's `mock.module` works fine because all consumers of the
 * framework resolve through the same module specifier.
 */

import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'

interface CallEntry {
  toolName: string
  args: Record<string, unknown>
}

interface FakeResult {
  isError: boolean
  content: Array<{ type: 'text'; text: string }>
  structuredContent?: unknown
}

const calls: CallEntry[] = []
const queued: FakeResult[] = []

function nextResult(toolName: string): FakeResult {
  const r = queued.shift()
  if (!r) {
    throw new Error(`tab-group-ops.test: no queued result for ${toolName}`)
  }
  return r
}

// We deliberately mock only `executeTool` (not BROWSER_TOOLS) because
// `mock.module` persists across files in the same `bun test` run, and
// the v2 dispatch suite needs the real BROWSER_TOOLS catalogue. The
// real `tab_groups` and `windows` ToolDefinition objects pass through
// our orchestrator untouched; we just short-circuit the dispatch.
mock.module('@browseros/browser-mcp/tools/framework', () => ({
  executeTool: async (def: { name: string }, args: Record<string, unknown>) => {
    calls.push({ toolName: def.name, args })
    return nextResult(def.name)
  },
}))

// IMPORTANT: dynamic import happens AFTER the module mocks above.
const { tabGroupTracker } = await import('../../src/lib/agent-tab-groups')
const {
  applyAgentTabGroupTitle,
  ensureAgentTabGroup,
  closeAgentTabGroupForAgent,
  focusAgentTabGroup,
} = await import('../../src/services/tab-group-ops')

const fakeSession = {} as never

function queue(...rs: FakeResult[]) {
  queued.push(...rs)
}

function ok(structured?: unknown): FakeResult {
  return {
    isError: false,
    content: [{ type: 'text', text: 'ok' }],
    structuredContent: structured,
  }
}

function err(msg: string): FakeResult {
  return {
    isError: true,
    content: [{ type: 'text', text: msg }],
  }
}

describe('ensureAgentTabGroup', () => {
  beforeEach(() => {
    calls.length = 0
    queued.length = 0
    tabGroupTracker.reset()
  })
  afterEach(() => {
    queued.length = 0
  })

  it('first open creates the group, locks the colour, and remembers groupId + windowId', async () => {
    queue(
      ok({ group: { groupId: 'G1', windowId: 42, title: 'claude-code' } }),
      ok(),
    )
    await ensureAgentTabGroup({
      agentId: 'claude-code',
      slug: 'claude-code',
      pageId: 1,
      session: fakeSession,
    })
    expect(calls.length).toBe(2)
    expect(calls[0]?.toolName).toBe('tab_groups')
    expect(calls[0]?.args).toMatchObject({
      action: 'create',
      pages: [1],
      title: 'claude-code',
    })
    expect(calls[1]?.args).toMatchObject({
      action: 'update',
      groupId: 'G1',
    })
    const record = tabGroupTracker.getByAgentId('claude-code')
    expect(record?.groupId).toBe('G1')
    expect(record?.windowId).toBe(42)
    expect(record?.color).toBeDefined()
  })

  it('subsequent opens add the new page to the existing group instead of creating a duplicate', async () => {
    queue(ok({ group: { groupId: 'G1', windowId: 42 } }), ok(), ok())
    await ensureAgentTabGroup({
      agentId: 'cursor',
      slug: 'cursor',
      pageId: 1,
      session: fakeSession,
    })
    await ensureAgentTabGroup({
      agentId: 'cursor',
      slug: 'cursor',
      pageId: 2,
      session: fakeSession,
    })
    expect(calls.length).toBe(3)
    // 1st call: create. 2nd: update (colour). 3rd: add to existing group.
    expect(calls[2]?.args).toMatchObject({
      action: 'create',
      groupId: 'G1',
      pages: [2],
    })
  })

  it('swallows tab_groups create errors so the tabs new path is never blocked', async () => {
    queue(err('manager down'))
    await ensureAgentTabGroup({
      agentId: 'zed',
      slug: 'zed',
      pageId: 1,
      session: fakeSession,
    })
    const record = tabGroupTracker.getByAgentId('zed')
    expect(record).not.toBeNull()
    expect(record?.groupId).toBeNull()
  })

  it('forgets the stale groupId when add-to-group errors so the next call recreates', async () => {
    queue(
      ok({ group: { groupId: 'G1', windowId: 1 } }),
      ok(),
      err('group not found'),
    )
    await ensureAgentTabGroup({
      agentId: 'codex',
      slug: 'codex',
      pageId: 1,
      session: fakeSession,
    })
    await ensureAgentTabGroup({
      agentId: 'codex',
      slug: 'codex',
      pageId: 2,
      session: fakeSession,
    })
    expect(tabGroupTracker.getByAgentId('codex')?.groupId).toBeNull()
  })

  it('serialises near-simultaneous opens so only one create dispatches', async () => {
    queue(ok({ group: { groupId: 'G1', windowId: 1 } }), ok(), ok())
    await Promise.all([
      ensureAgentTabGroup({
        agentId: 'parallel',
        slug: 'parallel',
        pageId: 1,
        session: fakeSession,
      }),
      ensureAgentTabGroup({
        agentId: 'parallel',
        slug: 'parallel',
        pageId: 2,
        session: fakeSession,
      }),
    ])
    const creates = calls.filter(
      (c) =>
        c.toolName === 'tab_groups' &&
        (c.args as { action?: string }).action === 'create' &&
        (c.args as { groupId?: string }).groupId === undefined,
    )
    expect(creates.length).toBe(1)
  })
})

describe('closeAgentTabGroupForAgent', () => {
  beforeEach(() => {
    calls.length = 0
    queued.length = 0
    tabGroupTracker.reset()
  })

  it('closes the group when the ref count hits zero', async () => {
    queue(ok({ group: { groupId: 'G1', windowId: 1 } }), ok(), ok())
    await ensureAgentTabGroup({
      agentId: 'a',
      slug: 'a',
      pageId: 1,
      session: fakeSession,
    })
    tabGroupTracker.incrementSession('a')
    await closeAgentTabGroupForAgent({ agentId: 'a', session: fakeSession })
    const closeCall = calls.find(
      (c) => (c.args as { action?: string }).action === 'close',
    )
    expect(closeCall).toBeDefined()
    expect((closeCall?.args as { groupId?: string }).groupId).toBe('G1')
  })

  it('keeps the group alive when at least one other session still references the agentId', async () => {
    queue(ok({ group: { groupId: 'G1', windowId: 1 } }), ok())
    await ensureAgentTabGroup({
      agentId: 'a',
      slug: 'a',
      pageId: 1,
      session: fakeSession,
    })
    tabGroupTracker.incrementSession('a')
    tabGroupTracker.incrementSession('a')
    await closeAgentTabGroupForAgent({ agentId: 'a', session: fakeSession })
    const closeCall = calls.find(
      (c) => (c.args as { action?: string }).action === 'close',
    )
    expect(closeCall).toBeUndefined()
  })

  it('is a no-op when the agentId has no tracker record', async () => {
    await closeAgentTabGroupForAgent({ agentId: 'ghost', session: fakeSession })
    expect(calls.length).toBe(0)
  })
})

describe('applyAgentTabGroupTitle', () => {
  beforeEach(() => {
    calls.length = 0
    queued.length = 0
    tabGroupTracker.reset()
  })
  afterEach(() => {
    queued.length = 0
  })

  it('sets the title before first group create', async () => {
    tabGroupTracker.incrementSession('claude-code-abc123')
    await applyAgentTabGroupTitle({
      agentId: 'claude-code-abc123',
      title: 'claude/invoice-processing',
      session: null,
    })
    queue(ok({ group: { groupId: 'G1', windowId: 42 } }), ok())
    await ensureAgentTabGroup({
      agentId: 'claude-code-abc123',
      slug: 'claude-code',
      pageId: 1,
      session: fakeSession,
    })
    expect(calls[0]?.args).toMatchObject({
      action: 'create',
      pages: [1],
      title: 'claude/invoice-processing',
    })
  })

  it('updates an already-created group title', async () => {
    queue(ok({ group: { groupId: 'G1', windowId: 42 } }), ok())
    await ensureAgentTabGroup({
      agentId: 'cursor-abc123',
      slug: 'cursor',
      pageId: 1,
      session: fakeSession,
    })
    calls.length = 0
    queue(ok())
    await applyAgentTabGroupTitle({
      agentId: 'cursor-abc123',
      title: 'cursor/linkedin-jobs',
      session: fakeSession,
    })
    expect(calls).toHaveLength(1)
    expect(calls[0]?.args).toMatchObject({
      action: 'update',
      groupId: 'G1',
      title: 'cursor/linkedin-jobs',
    })
  })

  it('resolves without dispatch when no tracker record exists', async () => {
    await applyAgentTabGroupTitle({
      agentId: 'missing',
      title: 'claude/invoice-processing',
      session: fakeSession,
    })
    expect(calls).toEqual([])
  })

  it('resolves when title update returns an error', async () => {
    queue(ok({ group: { groupId: 'G1', windowId: 42 } }), ok())
    await ensureAgentTabGroup({
      agentId: 'claude-code-abc123',
      slug: 'claude-code',
      pageId: 1,
      session: fakeSession,
    })
    calls.length = 0
    queue(err('update failed'))
    await expect(
      applyAgentTabGroupTitle({
        agentId: 'claude-code-abc123',
        title: 'claude/invoice-processing',
        session: fakeSession,
      }),
    ).resolves.toBeUndefined()
    expect(calls[0]?.args).toMatchObject({
      action: 'update',
      groupId: 'G1',
      title: 'claude/invoice-processing',
    })
  })
})

describe('focusAgentTabGroup', () => {
  beforeEach(() => {
    calls.length = 0
    queued.length = 0
    tabGroupTracker.reset()
  })

  it('returns ok=false when the tracker has no group for the agent', async () => {
    const r = await focusAgentTabGroup({
      agentId: 'unknown',
      session: fakeSession,
    })
    expect(r.ok).toBe(false)
  })

  it('expands the group and activates the window for a tracked agent', async () => {
    queue(ok({ group: { groupId: 'G1', windowId: 42 } }), ok(), ok(), ok())
    await ensureAgentTabGroup({
      agentId: 'a',
      slug: 'a',
      pageId: 1,
      session: fakeSession,
    })
    const r = await focusAgentTabGroup({
      agentId: 'a',
      session: fakeSession,
    })
    expect(r.ok).toBe(true)
    expect(r.groupId).toBe('G1')
    expect(r.windowId).toBe(42)
    const expand = calls.find(
      (c) =>
        (c.args as { action?: string }).action === 'update' &&
        (c.args as { collapsed?: boolean }).collapsed === false,
    )
    expect(expand).toBeDefined()
    const activate = calls.find(
      (c) =>
        c.toolName === 'windows' &&
        (c.args as { action?: string }).action === 'activate',
    )
    expect(activate).toBeDefined()
    expect((activate?.args as { windowId?: number }).windowId).toBe(42)
  })
})
