/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js'

interface FrameworkCall {
  tool: string
  args: Record<string, unknown>
}

const frameworkCalls: FrameworkCall[] = []
let nextPage = 0

const realFramework = await import('@browseros/browser-mcp/tools/framework')
mock.module('@browseros/browser-mcp/tools/framework', () => ({
  ...realFramework,
  executeTool: async (
    tool: { name: string },
    args: Record<string, unknown>,
  ) => {
    frameworkCalls.push({ tool: tool.name, args })
    if (tool.name === 'tabs' && args.action === 'new') {
      nextPage += 1
      return {
        isError: false,
        content: [{ type: 'text', text: `opened ${nextPage}` }],
        structuredContent: { page: nextPage },
      }
    }
    if (tool.name === 'tab_groups' && args.action === 'create') {
      return {
        isError: false,
        content: [{ type: 'text', text: 'created' }],
        structuredContent: {
          group: { groupId: `G${nextPage}`, windowId: 1 },
        },
      }
    }
    return { isError: false, content: [{ type: 'text', text: 'ok' }] }
  },
}))

const { ownershipStore } = await import('../../src/domain/ownership')
const { setBrowserSession } = await import('../../src/lib/browser-session')
const {
  agentKeyFromClient,
  buildSessionGroupTitle,
  clientPrefixFromSlug,
  identityService,
} = await import('../../src/lib/mcp-session')
const { resetTabGroupEffectsForTesting } = await import(
  '../../src/mcp/effects/tab-groups'
)
const { resetSingleMcpInstanceForTesting } = await import(
  '../../src/mcp/single-server'
)
const { resetAuditDbForTesting, setAuditDbForTesting } = await import(
  '../../src/modules/db/db'
)
const app = (await import('../../src/server')).default

async function connect() {
  const transport = new StreamableHTTPClientTransport(
    new URL('http://localhost/mcp'),
    {
      fetch: ((input, init) =>
        app.fetch(new Request(input, init))) as typeof fetch,
    },
  )
  const client = new Client(
    { name: 'claude-code', version: '1.0.0' },
    { capabilities: {} },
  )
  await client.connect(transport)
  const sessionId = transport.sessionId
  if (!sessionId) throw new Error('missing session id')
  const identity = identityService.getIdentity(sessionId)
  if (!identity) throw new Error('missing identity')
  return { client, identity }
}

function textOf(result: { content?: unknown }): string {
  return (
    (result.content as Array<{ text?: string }> | undefined)?.[0]?.text ?? ''
  )
}

describe('name_session', () => {
  beforeEach(() => {
    setAuditDbForTesting()
    frameworkCalls.length = 0
    nextPage = 0
    resetSingleMcpInstanceForTesting()
    resetTabGroupEffectsForTesting()
    identityService.clear()
    ownershipStore.clear()
    setBrowserSession(null)
  })

  afterEach(() => {
    resetSingleMcpInstanceForTesting()
    resetTabGroupEffectsForTesting()
    identityService.clear()
    ownershipStore.clear()
    setBrowserSession(null)
    resetAuditDbForTesting()
  })

  it('registers the expected schema and annotations', async () => {
    const { client } = await connect()
    const tool = (await client.listTools()).tools.find(
      (candidate) => candidate.name === 'name_session',
    )

    expect(tool).toMatchObject({
      name: 'name_session',
      annotations: {
        title: 'Name session',
        readOnlyHint: false,
        destructiveHint: false,
        idempotentHint: true,
      },
      inputSchema: {
        type: 'object',
        properties: { name: { type: 'string', maxLength: 64 } },
        required: ['name'],
      },
    })
    expect(tool?.description).toContain('small lowercase 2-3 word label')
    await client.close()
  })

  it('normalizes and renames while disconnected, returning old and new', async () => {
    const { client, identity } = await connect()
    const oldTitle = buildSessionGroupTitle(
      clientPrefixFromSlug(identity.slug),
      identity.generatedLabel,
    )
    const result = await client.callTool({
      name: 'name_session',
      arguments: { name: '  Invoice Processing!!!  ' },
    })

    expect(result.isError).toBeFalsy()
    expect(identity.label).toBe('invoice-processing')
    expect(textOf(result)).toBe(
      `renamed to claude/invoice-processing (was ${oldTitle})`,
    )
    expect(frameworkCalls).toEqual([])
    await client.close()
  })

  it('rejects a name that normalizes to empty', async () => {
    const { client, identity } = await connect()
    const result = await client.callTool({
      name: 'name_session',
      arguments: { name: '!!!' },
    })

    expect(result.isError).toBe(true)
    expect(identity.label).toBe(identity.generatedLabel)
    expect(textOf(result)).toContain('usable session name')
    await client.close()
  })

  it('renames before the first tab and creates the group with that title', async () => {
    const { client, identity } = await connect()
    await client.callTool({
      name: 'name_session',
      arguments: { name: 'Invoice Processing' },
    })
    setBrowserSession({
      pages: {
        getInfo: (pageId: number) => ({
          targetId: `target-${pageId}`,
          url: 'https://example.com',
          title: 'Example',
        }),
      },
    } as never)

    const result = await client.callTool({
      name: 'tabs',
      arguments: { action: 'new', url: 'https://example.com' },
    })
    await Bun.sleep(0)

    expect(result.isError).toBeFalsy()
    expect(textOf(result)).not.toContain('Tip:')
    expect(
      frameworkCalls.find(
        (call) => call.tool === 'tab_groups' && call.args.action === 'create',
      )?.args.title,
    ).toBe('claude/invoice-processing')
    expect(ownershipStore.groupOf(agentKeyFromClient(identity))?.title).toBe(
      'claude/invoice-processing',
    )
    ownershipStore.clear()
    await client.close()
  })
})
