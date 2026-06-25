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
 * The tools surface is the real `@browseros/browser-mcp` catalogue. Tool
 * dispatches that pass the permission gate hit the
 * "session not connected" short-circuit because the cockpit's
 * runtime is not yet bound to a live Chromium (that happens in a
 * later commit). The permission-gate paths (Auto / Block / Ask) are
 * fully exercisable without a session.
 */

import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js'
import type { NewAgentValues } from '../../src/routes/agents/schemas'
import * as agents from '../../src/routes/agents/service'
import app from '../../src/server'
import { withTempBrowserosDir } from '../_helpers/temp-browseros-dir'

// The legacy `/mcp/:slug` route is 404-gated by default. Each test
// here drives a per-agent MCP session, so the suite explicitly opts
// in to the legacy path. v2 single-MCP coverage lives in its own
// integration test.
let prevLegacyFlag: string | undefined
beforeAll(() => {
  // biome-ignore lint/style/noProcessEnv: legacy-route gate is sourced from process.env at request time; tests must drive it directly
  prevLegacyFlag = process.env.COCKPIT_LEGACY_PER_AGENT_MCP
  // biome-ignore lint/style/noProcessEnv: legacy-route gate is sourced from process.env at request time; tests must drive it directly
  process.env.COCKPIT_LEGACY_PER_AGENT_MCP = '1'
})
afterAll(() => {
  if (prevLegacyFlag === undefined) {
    // biome-ignore lint/style/noProcessEnv: legacy-route gate is sourced from process.env at request time; tests must drive it directly
    delete process.env.COCKPIT_LEGACY_PER_AGENT_MCP
  } else {
    // biome-ignore lint/style/noProcessEnv: legacy-route gate is sourced from process.env at request time; tests must drive it directly
    process.env.COCKPIT_LEGACY_PER_AGENT_MCP = prevLegacyFlag
  }
})

const REAL_CATALOGUE = [
  'act',
  'diff',
  'download',
  'evaluate',
  'grep',
  'navigate',
  'pdf',
  'read',
  'run',
  'screenshot',
  'snapshot',
  'tab_groups',
  'tabs',
  'upload',
  'wait',
  'windows',
] as const

function makeAgentInput(): NewAgentValues {
  return {
    name: 'Cowork . MCP smoke',
    harness: 'Claude Desktop',
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
  test('deleted agent slug starts 404-ing immediately on the next request', async () => {
    await withTempBrowserosDir(async () => {
      const created = await agents.create(makeAgentInput())
      const before = await app.fetch(
        new Request(`http://localhost/mcp/${created.slug}`, {
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
              clientInfo: { name: 'test', version: '0' },
            },
          }),
        }),
      )
      expect(before.status).toBe(200)

      await agents.remove(created.id)

      const after = await app.fetch(
        new Request(`http://localhost/mcp/${created.slug}`, {
          method: 'POST',
          headers: {
            'content-type': 'application/json',
            accept: 'application/json, text/event-stream',
          },
          body: JSON.stringify({
            jsonrpc: '2.0',
            id: 2,
            method: 'initialize',
            params: {
              protocolVersion: '2024-11-05',
              capabilities: {},
              clientInfo: { name: 'test', version: '0' },
            },
          }),
        }),
      )
      expect(after.status).toBe(404)
    })
  })

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

  test('tools/list returns the real ten-tool catalogue', async () => {
    await withTempBrowserosDir(async () => {
      const created = await agents.create(makeAgentInput())
      const client = await connectedClientFor(created.slug)
      const tools = await client.listTools()
      const names = tools.tools.map((t) => t.name).sort()
      expect(names).toEqual([...REAL_CATALOGUE])
      await client.close()
    })
  })

  test('navigate on the Auto path short-circuits with "session not connected"', async () => {
    await withTempBrowserosDir(async () => {
      const created = await agents.create(makeAgentInput())
      const client = await connectedClientFor(created.slug)
      const result = await client.callTool({
        name: 'navigate',
        arguments: { page: 0, action: 'url', url: 'https://docs.google.com' },
      })
      expect(result.isError).toBe(true)
      const content = result.content as Array<{ type: string; text: string }>
      expect(content[0].text).toContain('browser session not connected')
      await client.close()
    })
  })

  test('navigate on a site-rule blocked domain (Block verdict) returns a structured error', async () => {
    await withTempBrowserosDir(async () => {
      const created = await agents.create(makeAgentInput())
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
        arguments: { page: 0, action: 'url', url: 'https://docs.google.com' },
      })
      expect(result.isError).toBe(true)
      const content = result.content as Array<{ type: string; text: string }>
      expect(content[0].text).toContain('blocked by site-rule')
      expect(content[0].text).toContain('navigate')
      expect(content[0].text).toContain('docs.google.com')
      await client.close()
    })
  })

  test('navigate refuses javascript:, file:, and data: URIs at the cockpit layer', async () => {
    await withTempBrowserosDir(async () => {
      const created = await agents.create(makeAgentInput())
      const client = await connectedClientFor(created.slug)
      for (const url of [
        'javascript:alert(1)',
        'file:///etc/passwd',
        'data:text/html,<script>1</script>',
      ]) {
        const result = await client.callTool({
          name: 'navigate',
          arguments: { page: 0, action: 'url', url },
        })
        expect(result.isError).toBe(true)
        const content = result.content as Array<{ type: string; text: string }>
        expect(content[0].text).toContain('only http(s) is allowed')
      }
      await client.close()
    })
  })

  test('a verb whose agent verdict is Ask returns the deferred-approval error', async () => {
    await withTempBrowserosDir(async () => {
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
        arguments: { page: 0, action: 'url', url: 'https://docs.google.com' },
      })
      expect(result.isError).toBe(true)
      const content = result.content as Array<{ type: string; text: string }>
      expect(content[0].text).toContain('approval required for navigate')
      await client.close()
    })
  })
})
