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
 *     renders ownership in text while structured data stays internal.
 *   - Same-name sessions share durable ownership; different names remain isolated.
 *   - `tabs close` drops the page from the ledger.
 *   - A page-targeted dispatch with a foreign `page` id is rejected
 *     with the clean error BEFORE `executeTool` fires.
 *   - Ownership and groups survive idle reap and reconnect.
 */

import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import { existsSync } from 'node:fs'
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js'

interface CallEntry {
  toolName: string
  args: Record<string, unknown>
  defaultTabGroupId?: string
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
  executeTool: async (
    def: { name: string },
    args: Record<string, unknown>,
    context: { defaultTabGroupId?: string },
  ) => {
    calls.push({
      toolName: def.name,
      args,
      defaultTabGroupId: context.defaultTabGroupId,
    })
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

function expectNudgedText(
  content: Array<{ type: string; text?: string }> | undefined,
  expected: string,
): void {
  expect(content?.[0]?.text).toBe(expected)
  const tip = content?.[content.length - 1]
  expect(tip?.text).toStartWith('Tip: this session is "claude/')
  expect(tip?.text).toEndWith(
    ' — rename it with name_session name="<2-3 word task label>"',
  )
}

async function waitFor(predicate: () => boolean): Promise<void> {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (predicate()) return
    await Bun.sleep(2)
  }
  throw new Error('condition was not reached')
}

const { setBrowserSession } = await import('../../src/lib/browser-session')
const { ownershipStore } = await import('../../src/domain/ownership')
const { tabActivityRegistry } = await import('../../src/lib/tab-activity')
const { agentIdentityFromClient, agentKeyFromClient, identityService } =
  await import('../../src/lib/mcp-session')
const { resetTabGroupEffectsForTesting } = await import(
  '../../src/mcp/effects/tab-groups'
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
const { listDispatches } = await import('../../src/services/audit-log')
const { screencastCache } = await import('../../src/services/screencast-cache')
const { clearFirstCapturesForTesting, screenshotPath } = await import(
  '../../src/services/screenshots'
)
const { withTempBrowserClawDir } = await import(
  '../_helpers/temp-browserclaw-dir'
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
  const key = agentKeyFromClient(identity)
  return { client, sessionId, agentId, slug, key }
}

interface TabsListResult {
  isError?: boolean
  content?: Array<{ type: string; text?: string }>
  structuredContent?: unknown
}

const ORIGINAL_IDLE = env.sessionIdleMs

describe('per-agent tabs isolation', () => {
  beforeEach(() => {
    setAuditDbForTesting()
    calls.length = 0
    queued.length = 0
    resetSingleMcpInstanceForTesting()
    identityService.clear()
    ownershipStore.clear()
    resetTabGroupEffectsForTesting()
    tabActivityRegistry.clear()
    screencastCache.resetForTesting()
    clearFirstCapturesForTesting()
    env.sessionIdleMs = 50
  })
  afterEach(() => {
    queued.length = 0
    resetSingleMcpInstanceForTesting()
    identityService.clear()
    ownershipStore.clear()
    resetTabGroupEffectsForTesting()
    tabActivityRegistry.clear()
    screencastCache.resetForTesting()
    clearFirstCapturesForTesting()
    setBrowserSession(null)
    env.sessionIdleMs = ORIGINAL_IDLE
    resetAuditDbForTesting()
  })

  it('strips the wire result after audit, screenshot, ownership, and grouping effects', async () => {
    await withTempBrowserClawDir(async () => {
      stubSessionForPage(7, 'target-7')
      const jpegBase64 = Buffer.from('cached-jpeg').toString('base64')
      screencastCache.set(7, {
        jpegBase64,
        capturedAt: Date.now(),
        byteLength: Buffer.from(jpegBase64, 'base64').length,
      })
      queue(
        ok({ page: 7 }),
        ok({ group: { groupId: 'G1', windowId: 42 } }),
        ok(),
      )
      const { client, key, sessionId } = await connect('claude-code')

      const result = await client.callTool({
        name: 'tabs',
        arguments: { action: 'new', url: 'https://example.com/' },
      })

      expect(result.structuredContent).toBeUndefined()
      expect([...ownershipStore.pagesOf(key)]).toEqual([7])
      await waitFor(() => ownershipStore.groupOf(key)?.id === 'G1')
      const rows = listDispatches({ sessionId }).rows
      expect(rows).toHaveLength(1)
      const row = rows[0]
      if (!row) throw new Error('missing audit row')
      expect(JSON.parse(row.resultMeta).structuredKeys).toEqual(['page'])
      await waitFor(() => existsSync(screenshotPath(row.id)))
      expect(existsSync(screenshotPath(row.id))).toBe(true)

      await client.close()
    })
  })

  it('strips schema-shaped run output while preserving its text', async () => {
    setBrowserSession({ pages: { getInfo: () => undefined } } as never)
    queue({
      isError: false,
      content: [{ type: 'text', text: 'ok\nreturn: 42' }],
      structuredContent: { ok: true, value: 42, logs: [] },
    })
    const { client } = await connect('claude-code')

    const result = await client.callTool({
      name: 'run',
      arguments: { code: 'return 42' },
    })

    expect(result.structuredContent).toBeUndefined()
    expectNudgedText(
      result.content as Array<{ type: string; text?: string }>,
      'ok\nreturn: 42',
    )
    await client.close()
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
    const { key, client } = await connect('claude-code')
    await client.callTool({
      name: 'tabs',
      arguments: { action: 'new', url: 'https://news.google.com/' },
    })
    expect([...ownershipStore.pagesOf(key)]).toEqual([7])
    const listResult = (await client.callTool({
      name: 'tabs',
      arguments: { action: 'list' },
    })) as TabsListResult
    expect(listResult.isError).toBeFalsy()
    expect(listResult.structuredContent).toBeUndefined()
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
    expect(listA.structuredContent).toBeUndefined()
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
    expect(listB.structuredContent).toBeUndefined()
    const textB = (listB.content as Array<{ type: string; text?: string }>)?.[0]
      ?.text
    expect(textB).toContain('Your tabs:')
    expect(textB).toContain(', owned by claude-code')

    await a.client.close()
    await b.client.close()
  })

  it('same-name sessions keep independent ownership and groups', async () => {
    stubSessionForPage(1, 't1', 'https://a.example/', 'A')
    queue(ok({ page: 1 }), ok({ group: { groupId: 'GA', windowId: 1 } }), ok())
    const a = await connect('claude-code')
    await a.client.callTool({
      name: 'tabs',
      arguments: { action: 'new', url: 'https://a.example/' },
    })

    const b = await connect('claude-code')
    expect(a.agentId).not.toBe(b.agentId)
    expect(a.key).not.toBe(b.key)
    expect(a.slug).toBe('claude-code')
    expect(b.slug).toBe('claude-code')

    calls.length = 0
    const snapshot = (await b.client.callTool({
      name: 'snapshot',
      arguments: { page: 1 },
    })) as TabsListResult
    expect(snapshot.isError).toBeTruthy()
    expect(calls).toEqual([])

    queue(
      ok({
        pages: [{ page: 1, url: 'https://a.example/', title: 'A' }],
      }),
    )
    const listB = (await b.client.callTool({
      name: 'tabs',
      arguments: { action: 'list' },
    })) as TabsListResult
    expect(listB.structuredContent).toBeUndefined()
    const textB = (listB.content as Array<{ type: string; text?: string }>)?.[0]
      ?.text
    expect(textB).toContain("Other agents' tabs:")
    expect(textB).toContain('owned by claude-code')

    const different = await connect('cursor')
    calls.length = 0
    const rejected = (await different.client.callTool({
      name: 'snapshot',
      arguments: { page: 1 },
    })) as TabsListResult
    expect(rejected.isError).toBeTruthy()
    expect(calls).toEqual([])

    await a.client.close()
    await b.client.close()
    await different.client.close()
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
    expect(list.structuredContent).toBeUndefined()
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
    expect(list.structuredContent).toBeUndefined()
    expectNudgedText(
      list.content as Array<{ type: string; text?: string }>,
      '(no open pages)',
    )
    await client.close()
  })

  it('prunes stale ownership before annotating a tabs list', async () => {
    setBrowserSession({
      pages: { getInfo: (_id: number) => undefined },
      // biome-ignore lint/suspicious/noExplicitAny: test stub
    } as any)
    const { client, key } = await connect('claude-code')
    ownershipStore.claimPage(key, 7)
    ownershipStore.claimPage(key, 8)
    queue(
      ok({ pages: [{ page: 7, url: 'https://live.example/', title: 'Live' }] }),
    )

    const list = (await client.callTool({
      name: 'tabs',
      arguments: { action: 'list' },
    })) as TabsListResult
    expect([...ownershipStore.pagesOf(key)]).toEqual([7])
    expect(ownershipStore.ownerOf(8)).toBeNull()
    expect(list.structuredContent).toBeUndefined()
    const text = (list.content as Array<{ type: string; text?: string }>)?.[0]
      ?.text
    expect(text).toContain('Your tabs:')
    expect(text).toContain('[7] https://live.example/ (Live)')
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
    const { key, client } = await connect('claude-code')
    await client.callTool({
      name: 'tabs',
      arguments: { action: 'new', url: 'https://x.com/' },
    })
    expect([...ownershipStore.pagesOf(key)]).toEqual([5])
    // tabs close takes `page` as input so the
    // cross-agent guard sees the ownership check pass for our own
    // page.
    await client.callTool({
      name: 'tabs',
      arguments: { action: 'close', page: 5 },
    })
    expect([...ownershipStore.pagesOf(key)]).toEqual([])
    await client.close()
  })

  it('opens later tabs directly in the durable group', async () => {
    setBrowserSession({
      pages: {
        getInfo: (id: number) =>
          id === 1 || id === 2
            ? {
                targetId: `t${id}`,
                url: 'https://x.com/',
                title: 'X',
                groupId: id === 2 ? 'G' : undefined,
              }
            : undefined,
      },
      // biome-ignore lint/suspicious/noExplicitAny: test stub
    } as any)
    queue(
      ok({ page: 1 }),
      ok({ group: { groupId: 'G', windowId: 1 } }),
      ok(),
      ok({ page: 2 }),
    )
    const { client } = await connect('claude-code')
    await client.callTool({
      name: 'tabs',
      arguments: { action: 'new', url: 'https://one.example/' },
    })
    await client.callTool({
      name: 'tabs',
      arguments: { action: 'new', url: 'https://two.example/' },
    })

    const tabCreates = calls.filter(
      (call) => call.toolName === 'tabs' && call.args.action === 'new',
    )
    expect(tabCreates).toHaveLength(2)
    expect(tabCreates[0]?.defaultTabGroupId).toBeUndefined()
    expect(tabCreates[1]?.defaultTabGroupId).toBe('G')
    expect(
      calls.filter(
        (call) =>
          call.toolName === 'tab_groups' && call.args.action === 'create',
      ),
    ).toHaveLength(1)
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

  it('idle end retains old ownership but a returning conversation starts fresh', async () => {
    stubSessionForPage(7, 't7')
    queue(ok({ page: 7 }), ok({ group: { groupId: 'G', windowId: 1 } }), ok())
    const first = await connect('claude-code')
    await first.client.callTool({
      name: 'tabs',
      arguments: { action: 'new', url: 'https://x.com/' },
    })
    expect([...ownershipStore.pagesOf(first.key)]).toEqual([7])
    expect(ownershipStore.groupOf(first.key)?.id).toBe('G')

    // Reap the session.
    queue(ok())
    setLastActivityForTesting(first.sessionId, Date.now() - 10_000)
    sweepIdleSessions(Date.now())
    await Bun.sleep(0)
    expect([...ownershipStore.pagesOf(first.key)]).toEqual([7])
    expect(ownershipStore.groupOf(first.key)?.id).toBe('G')
    expect(ownershipStore.groupOf(first.key)?.collapsed).toBe(true)
    expect(calls.some((call) => call.args.action === 'close')).toBe(false)

    const second = await connect('claude-code')
    expect(second.key).not.toBe(first.key)
    calls.length = 0
    const snapshot = await second.client.callTool({
      name: 'snapshot',
      arguments: { page: 7 },
    })
    expect(snapshot.isError).toBeTruthy()
    expect(calls).toEqual([])
    expect(ownershipStore.groupOf(first.key)?.collapsed).toBe(true)

    queue(
      ok({
        pages: [
          { page: 7, url: 'https://x.com/', title: 'Stale' },
          { page: 99, url: 'https://operator.example/', title: 'Op' },
        ],
      }),
    )
    const list = (await second.client.callTool({
      name: 'tabs',
      arguments: { action: 'list' },
    })) as TabsListResult
    expect(list.structuredContent).toBeUndefined()
    const text = (list.content as Array<{ type: string; text?: string }>)?.[0]
      ?.text
    expect(text).toContain("User's tabs:")
    expect(text).toContain("Other agents' tabs:")
    expect(text).toContain(
      `owned by ${ownershipStore.groupOf(first.key)?.title}`,
    )
    expect(text).not.toContain('Your tabs:')
    await second.client.close()
  })
})
