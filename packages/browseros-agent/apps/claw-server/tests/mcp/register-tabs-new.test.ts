/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Pins the registry-write side effect of the `tabs new` dispatch
 * branch in mcp/register.ts. Pre-fix, the registry only learned
 * about a tab from a subsequent page-targeted dispatch (snapshot,
 * navigate, etc) because `extractPageId` reads `page` from input
 * args, and `tabs new` carries no `page` in its input (the page id
 * is born in the result). Post-fix, the result-side pageId is
 * also written to the registry so `/tabs/activity` reflects the
 * tab the moment it opens.
 *
 * Pattern mirrors tab-group-ops.test.ts: mock `executeTool` at the
 * module boundary so the orchestrator drives synthetic results,
 * stub the browser session so `pages.getInfo(pageId)` returns a
 * known targetId, and assert against the singleton registry.
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
    throw new Error(`register-tabs-new.test: no queued result for ${toolName}`)
  }
  return r
}

// Preserve the framework's other exports (defineTool, textResult,
// errorResult, etc.) since `server.ts` -> `register.ts` ->
// `BROWSER_TOOLS` transitively imports every tool's source, and
// each tool imports helpers from this same module. Without the
// spread, those imports throw 'Export named "textResult" not found'
// when the catalogue loads.
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

// Imports must come AFTER mock.module so the orchestrator picks up
// the stubbed executeTool.
const { setBrowserSession } = await import('../../src/lib/browser-session')
const { tabActivityRegistry } = await import('../../src/lib/tab-activity')
const { agentIdentityFromClient, identityService } = await import(
  '../../src/lib/mcp-session'
)
const { resetSingleMcpInstanceForTesting } = await import(
  '../../src/mcp/single-server'
)
const app = (await import('../../src/server')).default

function stubSessionForPageId(pageId: number, targetId: string): void {
  setBrowserSession({
    pages: {
      getInfo: (id: number) =>
        id === pageId
          ? { targetId, url: 'https://example.com/', title: 'Ex' }
          : undefined,
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
  return { client, transport }
}

describe('mcp/register.ts: tabs new registry write', () => {
  beforeEach(() => {
    calls.length = 0
    queued.length = 0
    resetSingleMcpInstanceForTesting()
    identityService.clear()
    tabActivityRegistry.clear()
  })
  afterEach(() => {
    queued.length = 0
    resetSingleMcpInstanceForTesting()
    identityService.clear()
    tabActivityRegistry.clear()
    setBrowserSession(null)
  })

  it('records a registry entry from the result-derived page id when tabs new succeeds', async () => {
    stubSessionForPageId(7, 'target-7')
    // tabs new returns page=7 in structuredContent. ensureAgentTabGroup
    // also fires for action:'new' (one create + one update for colour)
    // so we queue those too.
    queue(ok({ page: 7 }), ok({ group: { groupId: 'G1', windowId: 42 } }), ok())
    const { client, transport } = await connect('codex-mcp-client')
    const result = await client.callTool({
      name: 'tabs',
      arguments: { action: 'new', url: 'https://example.com/' },
    })
    expect(result.isError).toBeFalsy()
    const identity = identityService.getIdentity(transport.sessionId as string)
    if (!identity) throw new Error('missing identity')
    const { agentId, slug } = agentIdentityFromClient(identity)
    expect(agentId).toMatch(/^codex-mcp-client-[0-9a-f]{6}$/)

    const snapshot = tabActivityRegistry.snapshot()
    expect(snapshot).toHaveLength(1)
    expect(snapshot[0]).toMatchObject({
      pageId: 7,
      targetId: 'target-7',
      agentId,
      slug,
      lastToolName: 'tabs',
    })

    await client.close()
  })

  it('does NOT record when the result has no page id (defensive)', async () => {
    stubSessionForPageId(7, 'target-7')
    queue(ok({})) // tabs new succeeded but the result lacks `page`
    const { client } = await connect('codex-mcp-client')
    const result = await client.callTool({
      name: 'tabs',
      arguments: { action: 'new', url: 'https://example.com/' },
    })
    expect(result.isError).toBeFalsy()

    expect(tabActivityRegistry.snapshot()).toEqual([])
    await client.close()
  })

  it('does NOT record from the result-derived path when action is not new', async () => {
    // The new code path is gated to action:'new' specifically; for
    // other actions, attribution still goes through extractPageId
    // (which requires `page` in input args).
    stubSessionForPageId(7, 'target-7')
    queue(ok({ tabs: [] }))
    const { client } = await connect('codex-mcp-client')
    const result = await client.callTool({
      name: 'tabs',
      arguments: { action: 'list' },
    })
    expect(result.isError).toBeFalsy()
    expect(tabActivityRegistry.snapshot()).toEqual([])
    await client.close()
  })

  it('does NOT record if the result-derived page id is missing from the live session', async () => {
    // session.pages.getInfo returns undefined for page 99: the live
    // CDP target list does not contain it (race between tabs new
    // returning and the page registering with the session). Skip
    // the write rather than insert a record with a placeholder
    // targetId.
    stubSessionForPageId(7, 'target-7')
    queue(
      ok({ page: 99 }),
      ok({ group: { groupId: 'G1', windowId: 42 } }),
      ok(),
    )
    const { client } = await connect('codex-mcp-client')
    const result = await client.callTool({
      name: 'tabs',
      arguments: { action: 'new', url: 'https://example.com/' },
    })
    expect(result.isError).toBeFalsy()
    expect(tabActivityRegistry.snapshot()).toEqual([])
    await client.close()
  })
})
