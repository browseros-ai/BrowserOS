/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js'
import { ElicitRequestSchema } from '@modelcontextprotocol/sdk/types.js'

interface ToolCall {
  toolName: string
  args: Record<string, unknown>
}

const toolCalls: ToolCall[] = []
let nextPageId = 0

const realFramework = await import('@browseros/browser-mcp/tools/framework')
mock.module('@browseros/browser-mcp/tools/framework', () => ({
  ...realFramework,
  executeTool: async (
    tool: { name: string },
    args: Record<string, unknown>,
  ) => {
    toolCalls.push({ toolName: tool.name, args })
    if (tool.name === 'tabs' && args.action === 'new') {
      nextPageId += 1
      return {
        isError: false,
        content: [{ type: 'text', text: 'ok' }],
        structuredContent: { page: nextPageId },
      }
    }
    if (
      tool.name === 'tab_groups' &&
      args.action === 'create' &&
      args.groupId === undefined
    ) {
      return {
        isError: false,
        content: [{ type: 'text', text: 'ok' }],
        structuredContent: { group: { groupId: 'G1', windowId: 1 } },
      }
    }
    return { isError: false, content: [{ type: 'text', text: 'ok' }] }
  },
}))

const { ownershipStore } = await import('../../src/domain/ownership')
const { setBrowserSession } = await import('../../src/lib/browser-session')
const { agentKeyFromClient, identityService } = await import(
  '../../src/lib/mcp-session'
)
const { logger } = await import('../../src/lib/logger')
const { resetTabGroupEffectsForTesting } = await import(
  '../../src/mcp/effects/tab-groups'
)
const { resetSessionNamingForTests } = await import(
  '../../src/mcp/session-naming'
)
const { resetSingleMcpInstanceForTesting } = await import(
  '../../src/mcp/single-server'
)
const { resetAuditDbForTesting, setAuditDbForTesting } = await import(
  '../../src/modules/db/db'
)
const app = (await import('../../src/server')).default

async function connect(
  client: Client,
): Promise<{ transport: StreamableHTTPClientTransport; sessionId: string }> {
  const transport = new StreamableHTTPClientTransport(
    new URL('http://localhost/mcp'),
    {
      fetch: ((input, init) =>
        app.fetch(new Request(input, init))) as typeof fetch,
    },
  )
  await client.connect(transport)
  const sessionId = transport.sessionId
  if (!sessionId) throw new Error('no session id assigned')
  return { transport, sessionId }
}

async function waitFor(predicate: () => boolean): Promise<void> {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (predicate()) return
    await Bun.sleep(1)
  }
  throw new Error('condition was not reached')
}

describe('session naming delivery', () => {
  beforeEach(() => {
    setAuditDbForTesting()
    toolCalls.length = 0
    nextPageId = 0
    resetSingleMcpInstanceForTesting()
    resetSessionNamingForTests()
    resetTabGroupEffectsForTesting()
    identityService.clear()
    ownershipStore.clear()
    setBrowserSession({
      pages: {
        getInfo: (pageId: number) => ({
          targetId: `target-${pageId}`,
          url: 'https://example.com/',
          title: 'Example',
        }),
      },
    } as never)
  })

  afterEach(() => {
    resetSingleMcpInstanceForTesting()
    resetSessionNamingForTests()
    resetTabGroupEffectsForTesting()
    identityService.clear()
    ownershipStore.clear()
    setBrowserSession(null)
    resetAuditDbForTesting()
  })

  it('delivers and applies naming during the first successful tabs new', async () => {
    const client = new Client(
      { name: 'claude-code', version: '1.0.0' },
      { capabilities: { elicitation: { form: {} } } },
    )
    let received = 0
    let callSettled = false
    let receivedBeforeCallSettled = false
    client.setRequestHandler(ElicitRequestSchema, async () => {
      received += 1
      receivedBeforeCallSettled = !callSettled
      return { action: 'accept', content: { name: 'Invoice Processing' } }
    })
    const { sessionId } = await connect(client)

    await client
      .callTool({
        name: 'tabs',
        arguments: { action: 'new', url: 'https://example.com/' },
      })
      .finally(() => {
        callSettled = true
      })

    const identity = identityService.getIdentity(sessionId)
    if (!identity) throw new Error('identity missing')
    const key = agentKeyFromClient(identity)
    await waitFor(
      () =>
        identityService.getIdentity(sessionId)?.sessionLabel ===
          'invoice-processing' &&
        ownershipStore.groupOf(key)?.title === 'claude/invoice-processing',
    )
    expect(received).toBe(1)
    expect(receivedBeforeCallSettled).toBe(true)
    expect(ownershipStore.groupOf(key)).toMatchObject({
      title: 'claude/invoice-processing',
      titleExplicit: true,
    })
    expect(
      toolCalls.some(
        (call) =>
          call.toolName === 'tab_groups' &&
          call.args.action === 'update' &&
          call.args.title === 'claude/invoice-processing',
      ),
    ).toBe(true)

    await client.callTool({
      name: 'tabs',
      arguments: { action: 'new', url: 'https://example.com/again' },
    })
    expect(received).toBe(1)
    ownershipStore.clear()
    await client.close()
  })

  it('logs once and never elicits when the client lacks capability', async () => {
    const client = new Client(
      { name: 'claude-code', version: '1.0.0' },
      { capabilities: {} },
    )
    const logs: Array<{ message: string; sessionId?: unknown }> = []
    const originalInfo = logger.info
    logger.info = (message, fields) => {
      if (message === 'mcp client lacks elicitation capability') {
        logs.push({ message, sessionId: fields?.sessionId })
      }
    }

    try {
      const { sessionId } = await connect(client)
      const identity = identityService.getIdentity(sessionId)
      if (!identity) throw new Error('identity missing')
      const key = agentKeyFromClient(identity)
      await client.callTool({
        name: 'tabs',
        arguments: { action: 'new', url: 'https://example.com/' },
      })
      await waitFor(() => ownershipStore.groupOf(key) !== null)
      await client.callTool({
        name: 'tabs',
        arguments: { action: 'new', url: 'https://example.com/again' },
      })

      expect(logs).toEqual([
        { message: 'mcp client lacks elicitation capability', sessionId },
      ])
      ownershipStore.clear()
      await client.close()
    } finally {
      logger.info = originalInfo
    }
  })
})
