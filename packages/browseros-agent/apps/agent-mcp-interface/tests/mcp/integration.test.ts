/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * MCP route integration smoke. Spins the SDK's Client against a
 * fetch override that routes every request through Hono's
 * `app.fetch`, so we never bind a port. Each test gets a fresh
 * tmp `<browserosDir>` so created agents don't leak.
 *
 * This is the Phase 2 spike: prove Bun + Hono + the SDK's Web
 * Standard transport actually compose end-to-end. The Phase 3
 * commit replaces the smoke `ping` tool with `navigate` and adds
 * permission-gate cases.
 */

import { describe, expect, test } from 'bun:test'
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js'
import type { NewAgentValues } from '../../src/routes/agents/schemas'
import * as agents from '../../src/routes/agents/service'
import app from '../../src/server'
import { withTempBrowserosDir } from '../_helpers/temp-browseros-dir'

function makeAgentInput(): NewAgentValues {
  return {
    name: 'Cowork . MCP smoke',
    harness: 'Claude Cowork',
    loginMode: 'profile',
    selectedSites: [],
    approvals: {
      submit: 'Ask',
      payment: 'Block',
      delete: 'Ask',
      upload: 'Ask',
      navigate: 'Auto',
      input: 'Auto',
    },
    aclRuleIds: [],
    customAclRules: [],
  }
}

async function connectedClientFor(slug: string): Promise<Client> {
  const transport = new StreamableHTTPClientTransport(
    new URL(`http://localhost/mcp/${slug}`),
    {
      fetch: ((input, init) =>
        app.fetch(new Request(input, init))) as typeof fetch,
    },
  )
  const client = new Client(
    { name: 'test-client', version: '0.0.1' },
    { capabilities: {} },
  )
  await client.connect(transport)
  return client
}

describe('/mcp/:slug route', () => {
  test('unknown slug returns 404 at the route layer', async () => {
    await withTempBrowserosDir(async () => {
      const res = await app.fetch(
        new Request('http://localhost/mcp/never-existed', {
          method: 'POST',
          headers: {
            'content-type': 'application/json',
            accept: 'application/json, text/event-stream',
          },
          body: JSON.stringify({
            jsonrpc: '2.0',
            id: 1,
            method: 'initialize',
            params: {
              protocolVersion: '2024-11-05',
              capabilities: {},
              clientInfo: { name: 'curl', version: '0' },
            },
          }),
        }),
      )
      expect(res.status).toBe(404)
    })
  })

  test('full handshake: initialize and tools/list returns the catalog', async () => {
    await withTempBrowserosDir(async () => {
      const created = await agents.create(makeAgentInput())
      const client = await connectedClientFor(created.slug)
      const tools = await client.listTools()
      const names = tools.tools.map((t) => t.name)
      expect(names).toContain('navigate')
      await client.close()
    })
  })

  test('navigate (Auto verdict) returns the stub observation', async () => {
    await withTempBrowserosDir(async () => {
      const created = await agents.create(makeAgentInput())
      const client = await connectedClientFor(created.slug)

      const result = await client.callTool({
        name: 'navigate',
        arguments: { url: 'https://docs.google.com' },
      })
      expect(result.isError).toBeFalsy()
      const content = result.content as Array<{ type: string; text: string }>
      expect(content[0].text).toContain(
        '(stub) navigated to https://docs.google.com',
      )

      await client.close()
    })
  })

  test('navigate on a site-rule blocked domain (Block verdict) returns a structured error', async () => {
    await withTempBrowserosDir(async () => {
      const created = await agents.create(makeAgentInput())
      // Block navigation on any *.google.com via a site rule.
      const { add: addSiteRule } = await import(
        '../../src/routes/site-rules/service'
      )
      await addSiteRule({
        label: 'no google',
        domain: '*.google.com',
        action: 'navigate',
      })
      const client = await connectedClientFor(created.slug)
      const result = await client.callTool({
        name: 'navigate',
        arguments: { url: 'https://docs.google.com' },
      })
      expect(result.isError).toBe(true)
      const content = result.content as Array<{ type: string; text: string }>
      expect(content[0].text).toContain('blocked by site-rule')
      expect(content[0].text).toContain('navigate')
      expect(content[0].text).toContain('docs.google.com')
      await client.close()
    })
  })

  test('a verb whose agent verdict is Ask returns the deferred-approval error', async () => {
    await withTempBrowserosDir(async () => {
      // The default agent's navigate is Auto; flip it to Ask to
      // exercise the deferred path through the same code path.
      const askAgent = await agents.create({
        ...makeAgentInput(),
        name: 'Cowork . MCP ask',
        approvals: {
          ...makeAgentInput().approvals,
          navigate: 'Ask',
        },
      })
      const client = await connectedClientFor(askAgent.slug)
      const result = await client.callTool({
        name: 'navigate',
        arguments: { url: 'https://docs.google.com' },
      })
      expect(result.isError).toBe(true)
      const content = result.content as Array<{ type: string; text: string }>
      expect(content[0].text).toContain('approval required for navigate')
      await client.close()
    })
  })
})
