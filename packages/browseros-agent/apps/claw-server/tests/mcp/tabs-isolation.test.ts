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
 *   - Two connected sessions are isolated: one agent's list does
 *     NOT include the other's pages, even when both clients use the
 *     same name.
 *   - `tabs close` drops the page from the ledger.
 *   - A page-targeted dispatch with a foreign `page` id is rejected
 *     with the clean error BEFORE `executeTool` fires.
 *   - After the session is reaped (cleanupSessionState via the idle
 *     sweeper), the session-scoped ledger is empty; a new session
 *     with the same client name sees `(no open pages)`.
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
const { agentIdentityFromClient, identityService } = await import(
  '../../src/lib/mcp-session'
)
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
  const identity = identityService.getIdentity(sessionId)
  if (!identity) throw new Error('no identity registered')
  const { agentId, slug } = agentIdentityFromClient(identity)
  return { client, sessionId, agentId, slug }
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

  it('tabs new populates the ledger; follow-up tabs list groups your tab and the user tab separately', async () => {
    stubSessionForPage(7, 'target-7', 'https://news.google.com/', 'News')
    // Queue: tabs new -> ensureAgentTabGroup fires create + update ->
    // then tabs list underlying result contains BOTH the agent's tab
    // AND the operator's tab. The cockpit annotator keeps both and
    // groups them into `Your tabs:` and `User's tabs:` sections.
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
    const { agentId, client } = await connect('claude-code')
    await client.callTool({
      name: 'tabs',
      arguments: { action: 'new', url: 'https://news.google.com/' },
    })
    expect([...agentTabs.ownedBy(agentId)]).toEqual([7])
    const listResult = (await client.callTool({
      name: 'tabs',
      arguments: { action: 'list' },
    })) as TabsListResult
    expect(listResult.isError).toBeFalsy()
    expect(listResult.structuredContent?.pages).toEqual([
      {
        page: 7,
        url: 'https://news.google.com/',
        title: 'News',
        ownership: 'mine',
        ownerAgentId: agentId,
        ownerLabel: null,
      },
      {
        page: 42,
        url: 'https://operator.example/',
        title: "Op's tab",
        ownership: 'user',
        ownerAgentId: null,
        ownerLabel: null,
      },
    ])
    const text = (
      listResult.content as Array<{ type: string; text?: string }>
    )?.[0]?.text
    expect(text).toContain('Your tabs:')
    expect(text).toContain('[7] https://news.google.com/ (News)')
    expect(text).toContain("User's tabs:")
    expect(text).toContain("[42] https://operator.example/ (Op's tab)")
    await client.close()
  })

  it('two agents see each other tabs as "other-agent" with the owner label', async () => {
    // Two connected sessions. agent-a opens page 1, agent-b opens
    // page 2. Both call tabs list. Each sees its OWN page under
    // "Your tabs:", the peer's page under "Other agents' tabs:" with
    // the peer's slug in ownerLabel, and the operator page under
    // "User's tabs:".
    stubSessionForPage(1, 't1', 'https://a.example/', 'A')
    queue(ok({ page: 1 }), ok({ group: { groupId: 'GA', windowId: 1 } }), ok())
    const a = await connect('claude-code')
    await a.client.callTool({
      name: 'tabs',
      arguments: { action: 'new', url: 'https://a.example/' },
    })

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
    queue(ok({ page: 2 }), ok({ group: { groupId: 'GB', windowId: 1 } }), ok())
    const b = await connect('cursor')
    await b.client.callTool({
      name: 'tabs',
      arguments: { action: 'new', url: 'https://b.example/' },
    })

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
    expect(listA.structuredContent?.pages).toEqual([
      {
        ...opTabs[0],
        ownership: 'mine',
        ownerAgentId: a.agentId,
        ownerLabel: null,
      },
      {
        ...opTabs[1],
        ownership: 'other-agent',
        ownerAgentId: b.agentId,
        ownerLabel: 'cursor',
      },
      { ...opTabs[2], ownership: 'user', ownerAgentId: null, ownerLabel: null },
    ])
    const textA = (listA.content as Array<{ type: string; text?: string }>)?.[0]
      ?.text
    expect(textA).toContain('Your tabs:')
    expect(textA).toContain("Other agents' tabs:")
    expect(textA).toContain(', owned by cursor')
    expect(textA).toContain("User's tabs:")

    queue(ok({ pages: opTabs }))
    const listB = (await b.client.callTool({
      name: 'tabs',
      arguments: { action: 'list' },
    })) as TabsListResult
    expect(listB.structuredContent?.pages).toEqual([
      {
        ...opTabs[0],
        ownership: 'other-agent',
        ownerAgentId: a.agentId,
        ownerLabel: 'claude-code',
      },
      {
        ...opTabs[1],
        ownership: 'mine',
        ownerAgentId: b.agentId,
        ownerLabel: null,
      },
      { ...opTabs[2], ownership: 'user', ownerAgentId: null, ownerLabel: null },
    ])

    await a.client.close()
    await b.client.close()
  })

  it('same-name sessions are isolated by session-scoped agentId', async () => {
    stubSessionForPage(1, 't1', 'https://a.example/', 'A')
    queue(ok({ page: 1 }), ok({ group: { groupId: 'GA', windowId: 1 } }), ok())
    const a = await connect('claude-code')
    await a.client.callTool({
      name: 'tabs',
      arguments: { action: 'new', url: 'https://a.example/' },
    })

    const b = await connect('claude-code')
    expect(a.agentId).not.toBe(b.agentId)
    expect(a.slug).toBe('claude-code')
    expect(b.slug).toBe('claude-code')

    calls.length = 0
    const snapshot = (await b.client.callTool({
      name: 'snapshot',
      arguments: { page: 1 },
    })) as TabsListResult
    expect(snapshot.isError).toBeTruthy()
    expect(
      (snapshot.content as Array<{ type: string; text?: string }>)?.[0]?.text,
    ).toContain('page 1 is not owned')
    expect(calls.length).toBe(0)

    queue(
      ok({
        pages: [{ page: 1, url: 'https://a.example/', title: 'A' }],
      }),
    )
    const listB = (await b.client.callTool({
      name: 'tabs',
      arguments: { action: 'list' },
    })) as TabsListResult
    // Page 1 shows up as an other-agent tab (session A's ownership)
    // even though the two sessions share the same client slug.
    expect(listB.structuredContent?.pages).toEqual([
      {
        page: 1,
        url: 'https://a.example/',
        title: 'A',
        ownership: 'other-agent',
        ownerAgentId: a.agentId,
        ownerLabel: 'claude-code',
      },
    ])
    const textB = (listB.content as Array<{ type: string; text?: string }>)?.[0]
      ?.text
    expect(textB).toContain("Other agents' tabs:")
    expect(textB).toContain(', owned by claude-code')

    await a.client.close()
    await b.client.close()
  })

  it('tabs list with no owned pages shows the operator tabs under "User\'s tabs:"', async () => {
    setBrowserSession({
      pages: {
        getInfo: (_id: number) => undefined,
      },
      // biome-ignore lint/suspicious/noExplicitAny: test stub
    } as any)
    // Underlying tool returns operator tabs; the annotator groups
    // them under "User's tabs:" instead of hiding them.
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
    expect(list.structuredContent?.pages).toEqual([
      {
        page: 100,
        url: 'https://only-operator.example/',
        title: 'Op',
        ownership: 'user',
        ownerAgentId: null,
        ownerLabel: null,
      },
    ])
    const text = (list.content as Array<{ type: string; text?: string }>)?.[0]
      ?.text
    expect(text).toContain("User's tabs:")
    expect(text).toContain('[100] https://only-operator.example/ (Op)')
    expect(text).not.toContain('Your tabs:')
    await client.close()
  })

  it('tabs list with truly zero open pages still renders "(no open pages)"', async () => {
    setBrowserSession({
      pages: {
        getInfo: (_id: number) => undefined,
      },
      // biome-ignore lint/suspicious/noExplicitAny: test stub
    } as any)
    queue(ok({ pages: [] }))
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
    const { agentId, client } = await connect('claude-code')
    await client.callTool({
      name: 'tabs',
      arguments: { action: 'new', url: 'https://x.com/' },
    })
    expect([...agentTabs.ownedBy(agentId)]).toEqual([5])
    // Now the operator (or the agent) closes page 5. The agentTabs
    // ledger drops it. NB: tabs close takes `page` as input so the
    // cross-agent guard sees the ownership check pass for our own
    // page.
    await client.callTool({
      name: 'tabs',
      arguments: { action: 'close', page: 5 },
    })
    expect([...agentTabs.ownedBy(agentId)]).toEqual([])
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
    expect([...agentTabs.ownedBy(first.agentId)]).toEqual([7])

    // Reap the session.
    setLastActivityForTesting(first.sessionId, Date.now() - 10_000)
    sweepIdleSessions(Date.now())
    expect(agentTabs.ownedBy(first.agentId).size).toBe(0)

    // A fresh session for the same clientName starts empty and its
    // tabs list should reflect that even though the underlying tool
    // returns operator-owned pages.
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
    // After the reap, ownership of page 7 was dropped, so the new
    // session sees both pages under "User's tabs:" rather than
    // remembering the stale ownership.
    expect(list.structuredContent?.pages).toEqual([
      {
        page: 7,
        url: 'https://x.com/',
        title: 'Stale',
        ownership: 'user',
        ownerAgentId: null,
        ownerLabel: null,
      },
      {
        page: 99,
        url: 'https://operator.example/',
        title: 'Op',
        ownership: 'user',
        ownerAgentId: null,
        ownerLabel: null,
      },
    ])
    const text = (list.content as Array<{ type: string; text?: string }>)?.[0]
      ?.text
    expect(text).toContain("User's tabs:")
    expect(text).not.toContain('Your tabs:')
    expect(text).not.toContain("Other agents' tabs:")
    await second.client.close()
  })
})
