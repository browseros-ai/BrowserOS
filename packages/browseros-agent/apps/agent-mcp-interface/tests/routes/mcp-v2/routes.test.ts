/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Walks the v2 single-MCP endpoint with the real SDK Client. Confirms
 * that the shared transport assigns a session id, captures the
 * connecting agent's `clientInfo` into the identity service, and
 * exposes the same browser-tool catalogue the legacy per-slug route
 * served. No browser session is bound, so tool dispatch surfaces the
 * "session not connected" short-circuit, which is enough to prove the
 * dispatch path picked up identity from `extra.sessionId`.
 */

import { afterEach, beforeEach, describe, expect, test } from 'bun:test'
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js'
import {
  agentIdentityFromClient,
  identityService,
} from '../../../src/lib/mcp-session'
import { resetSingleMcpInstanceForTesting } from '../../../src/mcp/single-server'
import app from '../../../src/server'

async function connect(clientName: string, clientVersion = '0.0.1') {
  const transport = new StreamableHTTPClientTransport(
    new URL('http://localhost/mcp'),
    {
      fetch: ((input, init) =>
        app.fetch(new Request(input, init))) as typeof fetch,
    },
  )
  const client = new Client(
    { name: clientName, version: clientVersion },
    { capabilities: {} },
  )
  await client.connect(transport)
  return { client, transport }
}

describe('POST /cockpit/mcp (single endpoint)', () => {
  beforeEach(() => {
    resetSingleMcpInstanceForTesting()
    identityService.clear()
  })
  afterEach(() => {
    resetSingleMcpInstanceForTesting()
    identityService.clear()
  })

  test('initialize captures clientInfo into the identity map', async () => {
    const { client, transport } = await connect('claude-code', '1.4.2')
    expect(identityService.size()).toBe(1)
    const sessionId = transport.sessionId
    expect(sessionId).toBeDefined()
    const identity = identityService.getIdentity(sessionId as string)
    expect(identity?.clientName).toBe('claude-code')
    expect(identity?.clientVersion).toBe('1.4.2')
    const bridge = identity ? agentIdentityFromClient(identity) : null
    expect(bridge?.agentId).toBe('claude-code')
    expect(bridge?.slug).toBe('claude-code')
    await client.close()
  })

  test('tools/list returns the same catalogue as the legacy route', async () => {
    const { client } = await connect('claude-code')
    const list = await client.listTools()
    const names = list.tools.map((t) => t.name).sort()
    expect(names).toContain('navigate')
    expect(names).toContain('tabs')
    expect(names).toContain('screenshot')
    expect(names.length).toBeGreaterThanOrEqual(10)
    await client.close()
  })

  test('navigate without a bound browser short-circuits with session-not-connected', async () => {
    const { client } = await connect('claude-code')
    const result = await client.callTool({
      name: 'navigate',
      arguments: { page: 0, action: 'url', url: 'https://example.com' },
    })
    expect(result.isError).toBe(true)
    const text =
      (result.content as Array<{ type: string; text?: string }>)?.[0]?.text ??
      ''
    expect(text.toLowerCase()).toContain('browser session not connected')
    await client.close()
  })

  test('an unknown mcp-session-id header is rejected with 404 without leaking a session', async () => {
    const res = await app.fetch(
      new Request('http://localhost/mcp', {
        method: 'POST',
        headers: {
          'content-type': 'application/json',
          accept: 'application/json, text/event-stream',
          'mcp-session-id': 'definitely-not-a-known-session',
        },
        body: JSON.stringify({
          jsonrpc: '2.0',
          id: 1,
          method: 'tools/list',
          params: {},
        }),
      }),
    )
    expect(res.status).toBe(404)
    const body = (await res.json()) as { error?: string; hint?: string }
    expect(body.error).toBe('unknown mcp-session-id')
    expect(body.hint).toContain('initialize')
    // No leak: nothing was added to the identity map.
    expect(identityService.size()).toBe(0)
  })

  test('navigate refuses javascript:/file:/data: at the cockpit layer', async () => {
    const { client } = await connect('claude-code')
    for (const url of [
      'javascript:alert(1)',
      'file:///etc/passwd',
      'data:text/html,hi',
    ]) {
      const result = await client.callTool({
        name: 'navigate',
        arguments: { page: 0, action: 'url', url },
      })
      expect(result.isError).toBe(true)
      const text =
        (result.content as Array<{ type: string; text?: string }>)?.[0]?.text ??
        ''
      expect(text).toContain('only http(s) is allowed')
    }
    await client.close()
  })
})
