/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Integration tests for the per-agent tabs isolation. Drives the
 * real v2 MCP dispatch handler via the SDK client + a mocked
 * executeTool, then asserts:
 *
 *   - `tabs new` populates the ledger; the follow-up `tabs list`
 *     returns only the newly-opened page (in both text and
 *     structured channels).
 *   - Two connected sessions with different agentIds are isolated:
 *     one agent's list does NOT include the other's pages.
 *   - `tabs close` drops the page from the ledger.
 *   - A page-targeted dispatch with a foreign `page` id is rejected
 *     with the clean error BEFORE `executeTool` fires.
 *   - After the session is reaped (cleanupSessionState via the idle
 *     sweeper), the agent's ledger is empty; a new session for the
 *     same agentId sees `(no open pages)`.
 */

import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js'

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
    throw new Error(`tabs-isolation.test: no queued result for ${toolName}`)
  }
  return r
}

// Preserve the framework's other exports so BROWSER_TOOLS loads.
const realFramework = await import('@browseros/browser-mcp/tools/framework')
mock.module('@browseros/browser-mcp/tools/framework', () => ({
  ...realFramework,
  executeTool: async (def: { name: string }, args: Record<string, unknown>) => {
    calls.push({ toolName: def.name, args })
    return nextResult(def.name)
  },
}))

function queue(...results: FakeResult[]): void {
  queued.push(...results)
}
function ok(structured?: unknown): FakeResult {
  return {
    isError: false,
    content: [{ type: 'text', text: 'ok' }],
    structuredContent: structured,
  }
}

const { setBrowserSession } = await import('../../src/lib/browser-session')
const { agentTabs } = await import('../../src/lib/agent-tabs')
const { tabActivityRegistry } = await import('../../src/lib/tab-activity')
const { tabGroupTracker } = await import('../../src/lib/agent-tab-groups')
const { identityService } = await import('../../src/lib/mcp-session')
const {
  resetSingleMcpInstanceForTesting,
  setLastActivityForTesting,
  sweepIdleSessions,
} = await import('../../src/mcp/single-server')
const { env } = await import('../../src/env')
const { setAuditDbForTesting, resetAuditDbForTesting } = await import(
  '../../src/modules/db/db'
)
const app = (await import('../../src/server')).default

function stubSessionForPage(
  pageId: number,
  targetId: string,
  url = 'https://example.com/',
  title = 'Ex',
): void {
  setBrowserSession({
    pages: {
      getInfo: (id: number) =>
        id === pageId ? { targetId, url, title } : undefined,
    },
    // biome-ignore lint/suspicious/noExplicitAny: test stub
  } as any)
}

async function connect(clientName: string) {
  const transport = new StreamableHTTPClientTransport(
    new URL('http://localhost/mcp'),
    {
      fetch: ((input, init) =>
        app.fetch(new Request(input, init))) as typeof fetch,
    },
  )
  const client = new Client(
    { name: clientName, version: '0.0.1' },
    { capabilities: {} },
  )
  await client.connect(transport)
  const sessionId = transport.sessionId
  if (!sessionId) throw new Error('no session id assigned')
  return { client, sessionId }
}

interface TabsListResult {
  isError?: boolean
  content?: Array<{ type: string; text?: string }>
  structuredContent?: {
    pages?: Array<{ page: number; url?: string; title?: string }>
  }
}

const ORIGINAL_IDLE = env.sessionIdleMs

describe('per-agent tabs isolation', () => {
  beforeEach(() => {
    setAuditDbForTesting()
    calls.length = 0
    queued.length = 0
    resetSingleMcpInstanceForTesting()
    identityService.clear()
    tabGroupTracker.reset()
    tabActivityRegistry.clear()
    agentTabs.clear()
    env.sessionIdleMs = 50
  })
  afterEach(() => {
    queued.length = 0
    resetSingleMcpInstanceForTesting()
    identityService.clear()
    tabGroupTracker.reset()
    tabActivityRegistry.clear()
    agentTabs.clear()
    setBrowserSession(null)
    env.sessionIdleMs = ORIGINAL_IDLE
    resetAuditDbForTesting()
  })

  it('tabs new populates the ledger; follow-up tabs list returns only that page', async () => {
    stubSessionForPage(7, 'target-7', 'https://news.google.com/', 'News')
    // Queue: tabs new -> ensureAgentTabGroup fires create + update ->
    // then tabs list underlying result contains BOTH the agent's tab
    // AND the operator's tab. The cockpit filter should reduce it.
    queue(
      ok({ page: 7 }), // tabs new
      ok({ group: { groupId: 'G1', windowId: 42 } }), // tab_groups create
      ok(), // tab_groups update (colour)
      ok({
        pages: [
          { page: 7, url: 'https://news.google.com/', title: 'News' },
          { page: 42, url: 'https://operator.example/', title: "Op's tab" },
        ],
      }),
    )
    const { client } = await connect('claude-code')
    await client.callTool({
      name: 'tabs',
      arguments: { action: 'new', url: 'https://news.google.com/' },
    })
    expect([...agentTabs.ownedBy('claude-code')]).toEqual([7])
    const listResult = (await client.callTool({
      name: 'tabs',
      arguments: { action: 'list' },
    })) as TabsListResult
    expect(listResult.isError).toBeFalsy()
    expect(listResult.structuredContent?.pages).toEqual([
      { page: 7, url: 'https://news.google.com/', title: 'News' },
    ])
    const text = (
      listResult.content as Array<{ type: string; text?: string }>
    )?.[0]?.text
    expect(text).toContain('[7] https://news.google.com/')
    expect(text).not.toContain("Op's tab")
    expect(text).not.toContain('operator.example')
    await client.close()
  })

  it('two agents are isolated: one agent list does not include the other agent pages', async () => {
    // Two connected sessions.
    // agent-a opens page 1, agent-b opens page 2.
    // Both call tabs list and see only their own.
    stubSessionForPage(1, 't1', 'https://a.example/', 'A')
    // agent-a: tabs new (page 1) + tab_groups create + update
    queue(ok({ page: 1 }), ok({ group: { groupId: 'GA', windowId: 1 } }), ok())
    const a = await connect('claude-code')
    await a.client.callTool({
      name: 'tabs',
      arguments: { action: 'new', url: 'https://a.example/' },
    })

    // Switch session stub to a page id both agents can see (agent-b
    // opens page 2 while page 1 also exists). We stub pages 1 and 2.
    setBrowserSession({
      pages: {
        getInfo: (id: number) =>
          id === 1
            ? {
                targetId: 't1',
                url: 'https://a.example/',
                title: 'A',
              }
            : id === 2
              ? {
                  targetId: 't2',
                  url: 'https://b.example/',
                  title: 'B',
                }
              : undefined,
      },
      // biome-ignore lint/suspicious/noExplicitAny: test stub
    } as any)
    // agent-b: tabs new (page 2) + tab_groups create + update
    queue(ok({ page: 2 }), ok({ group: { groupId: 'GB', windowId: 1 } }), ok())
    const b = await connect('cursor')
    await b.client.callTool({
      name: 'tabs',
      arguments: { action: 'new', url: 'https://b.example/' },
    })

    // Underlying tabs list returns BOTH pages plus operator tab 99.
    // Filter must reduce per-agent.
    const opTabs = [
      { page: 1, url: 'https://a.example/', title: 'A' },
      { page: 2, url: 'https://b.example/', title: 'B' },
      { page: 99, url: 'https://operator.example/', title: 'Op' },
    ]
    queue(ok({ pages: opTabs }))
    const listA = (await a.client.callTool({
      name: 'tabs',
      arguments: { action: 'list' },
    })) as TabsListResult
    expect(listA.structuredContent?.pages).toEqual([opTabs[0]])

    queue(ok({ pages: opTabs }))
    const listB = (await b.client.callTool({
      name: 'tabs',
      arguments: { action: 'list' },
    })) as TabsListResult
    expect(listB.structuredContent?.pages).toEqual([opTabs[1]])

    await a.client.close()
    await b.client.close()
  })

  it('tabs list on an empty ledger returns "(no open pages)"', async () => {
    setBrowserSession({
      pages: {
        getInfo: (_id: number) => undefined,
      },
      // biome-ignore lint/suspicious/noExplicitAny: test stub
    } as any)
    // Underlying tool returns some operator tabs; filter drops them.
    queue(
      ok({
        pages: [
          { page: 100, url: 'https://only-operator.example/', title: 'Op' },
        ],
      }),
    )
    const { client } = await connect('claude-code')
    const list = (await client.callTool({
      name: 'tabs',
      arguments: { action: 'list' },
    })) as TabsListResult
    expect(list.structuredContent?.pages).toEqual([])
    expect(
      (list.content as Array<{ type: string; text?: string }>)?.[0]?.text,
    ).toBe('(no open pages)')
    await client.close()
  })

  it('tabs close drops the page from the ledger', async () => {
    stubSessionForPage(5, 't5')
    queue(
      ok({ page: 5 }), // tabs new
      ok({ group: { groupId: 'G', windowId: 1 } }),
      ok(),
      ok(), // tabs close
    )
    const { client } = await connect('claude-code')
    await client.callTool({
      name: 'tabs',
      arguments: { action: 'new', url: 'https://x.com/' },
    })
    expect([...agentTabs.ownedBy('claude-code')]).toEqual([5])
    // Now the operator (or the agent) closes page 5. The agentTabs
    // ledger drops it. NB: tabs close takes `page` as input so the
    // cross-agent guard sees the ownership check pass for our own
    // page.
    await client.callTool({
      name: 'tabs',
      arguments: { action: 'close', page: 5 },
    })
    expect([...agentTabs.ownedBy('claude-code')]).toEqual([])
    await client.close()
  })

  it('foreign page argument is rejected BEFORE executeTool fires', async () => {
    stubSessionForPage(7, 't7')
    // Agent opens page 7.
    queue(ok({ page: 7 }), ok({ group: { groupId: 'G', windowId: 1 } }), ok())
    const { client } = await connect('claude-code')
    await client.callTool({
      name: 'tabs',
      arguments: { action: 'new', url: 'https://x.com/' },
    })
    calls.length = 0 // reset the call log; we want to prove nothing new fires
    // Attempt snapshot on FOREIGN page 42. Guard should short-circuit.
    // We deliberately do NOT queue a result; if executeTool fires the
    // test will throw ("no queued result").
    const result = (await client.callTool({
      name: 'snapshot',
      arguments: { page: 42 },
    })) as TabsListResult
    expect(result.isError).toBeTruthy()
    expect(
      (result.content as Array<{ type: string; text?: string }>)?.[0]?.text,
    ).toContain('page 42 is not owned')
    expect(calls.length).toBe(0) // executeTool NEVER fired
    await client.close()
  })

  it('agent can still snapshot its OWN page (guard does not overreach)', async () => {
    stubSessionForPage(7, 't7')
    queue(
      ok({ page: 7 }),
      ok({ group: { groupId: 'G', windowId: 1 } }),
      ok(),
      ok({ someSnapshotShape: true }), // snapshot result
    )
    const { client } = await connect('claude-code')
    await client.callTool({
      name: 'tabs',
      arguments: { action: 'new', url: 'https://x.com/' },
    })
    const result = (await client.callTool({
      name: 'snapshot',
      arguments: { page: 7 },
    })) as TabsListResult
    expect(result.isError).toBeFalsy()
    await client.close()
  })

  it('idle reap forgets the agent ledger; next session sees an empty list', async () => {
    stubSessionForPage(7, 't7')
    queue(ok({ page: 7 }), ok({ group: { groupId: 'G', windowId: 1 } }), ok())
    const first = await connect('claude-code')
    await first.client.callTool({
      name: 'tabs',
      arguments: { action: 'new', url: 'https://x.com/' },
    })
    expect([...agentTabs.ownedBy('claude-code')]).toEqual([7])

    // Reap the session.
    setLastActivityForTesting(first.sessionId, Date.now() - 10_000)
    sweepIdleSessions(Date.now())
    expect(agentTabs.ownedBy('claude-code').size).toBe(0)

    // A fresh session for the same agentId (same clientName) starts
    // empty and its tabs list should reflect that even though the
    // underlying tool returns operator-owned pages.
    queue(
      ok({
        pages: [
          { page: 7, url: 'https://x.com/', title: 'Stale' },
          { page: 99, url: 'https://operator.example/', title: 'Op' },
        ],
      }),
    )
    const second = await connect('claude-code')
    const list = (await second.client.callTool({
      name: 'tabs',
      arguments: { action: 'list' },
    })) as TabsListResult
    expect(list.structuredContent?.pages).toEqual([])
    expect(
      (list.content as Array<{ type: string; text?: string }>)?.[0]?.text,
    ).toBe('(no open pages)')
    await second.client.close()
  })
})
