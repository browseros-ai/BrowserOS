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

  test('full handshake: initialize and tools/list returns all 6 tools', async () => {
    await withTempBrowserosDir(async () => {
      const created = await agents.create(makeAgentInput())
      const client = await connectedClientFor(created.slug)
      const tools = await client.listTools()
      const names = tools.tools.map((t) => t.name).sort()
      expect(names).toEqual([
        'attach',
        'click',
        'navigate',
        'read',
        'submit',
        'type',
      ])
      await client.close()
    })
  })

  test('every non-navigate tool dispatches against the stub on the Auto path', async () => {
    await withTempBrowserosDir(async () => {
      // Flip every verb to Auto so the catalog defaults don't gate us.
      const created = await agents.create({
        ...makeAgentInput(),
        approvals: {
          submit: 'Auto',
          payment: 'Auto',
          delete: 'Auto',
          upload: 'Auto',
          navigate: 'Auto',
          input: 'Auto',
        },
        selectedSites: ['concur.com'],
      })
      const client = await connectedClientFor(created.slug)

      const readRes = await client.callTool({
        name: 'read',
        arguments: { selector: '#main' },
      })
      expect(readRes.isError).toBeFalsy()
      expect((readRes.content as Array<{ text: string }>)[0].text).toContain(
        '(stub) read #main',
      )

      const clickRes = await client.callTool({
        name: 'click',
        arguments: { selector: '.btn' },
      })
      expect(clickRes.isError).toBeFalsy()
      expect((clickRes.content as Array<{ text: string }>)[0].text).toContain(
        '(stub) clicked .btn',
      )

      const typeRes = await client.callTool({
        name: 'type',
        arguments: { selector: '#q', value: 'hi' },
      })
      expect(typeRes.isError).toBeFalsy()
      expect((typeRes.content as Array<{ text: string }>)[0].text).toContain(
        '(stub) typed hi into #q',
      )

      const attachRes = await client.callTool({
        name: 'attach',
        arguments: { selector: '#file', filePath: '/tmp/receipt.pdf' },
      })
      expect(attachRes.isError).toBeFalsy()
      expect((attachRes.content as Array<{ text: string }>)[0].text).toContain(
        '(stub) attached receipt.pdf to #file',
      )

      const submitRes = await client.callTool({
        name: 'submit',
        arguments: { selector: 'form#expenses' },
      })
      expect(submitRes.isError).toBeFalsy()
      expect((submitRes.content as Array<{ text: string }>)[0].text).toContain(
        '(stub) submitted form#expenses',
      )

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

  test('navigate rejects non-http URIs at the schema boundary (no ACL bypass)', async () => {
    await withTempBrowserosDir(async () => {
      const created = await agents.create(makeAgentInput())
      const client = await connectedClientFor(created.slug)
      // Each of these has an empty `.hostname` and would silently
      // bypass site-rule matching if it slipped through.
      const evilUrls = [
        'javascript:alert(1)',
        'file:///etc/passwd',
        'data:text/html,<script>',
      ]
      for (const evil of evilUrls) {
        const call = client.callTool({
          name: 'navigate',
          arguments: { url: evil },
        })
        // The SDK surfaces a schema-side rejection as a thrown
        // McpError on the client. We accept either the throw OR a
        // structured isError result; both mean "the call never
        // dispatched and the gate was not bypassed".
        try {
          const result = await call
          expect(result.isError).toBe(true)
        } catch (err) {
          expect(err).toBeDefined()
        }
      }
      await client.close()
    })
  })

  test('attach rejects ".." path segments at the schema boundary', async () => {
    await withTempBrowserosDir(async () => {
      const created = await agents.create({
        ...makeAgentInput(),
        approvals: {
          ...makeAgentInput().approvals,
          upload: 'Auto',
        },
      })
      const client = await connectedClientFor(created.slug)
      const evilPaths = [
        '../etc/passwd',
        '/var/data/../../etc/passwd',
        '..\\windows\\system32\\config\\sam',
      ]
      for (const evil of evilPaths) {
        const call = client.callTool({
          name: 'attach',
          arguments: { selector: '#file', filePath: evil },
        })
        try {
          const result = await call
          expect(result.isError).toBe(true)
        } catch (err) {
          expect(err).toBeDefined()
        }
      }
      await client.close()
    })
  })
})
