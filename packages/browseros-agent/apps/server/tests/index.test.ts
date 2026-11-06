/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, it } from 'bun:test'
import assert from 'node:assert/strict'
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js'
import { CallToolResultSchema } from '@modelcontextprotocol/sdk/types.js'
import { withE2e } from './__helpers__/e2e'

describe('server end-to-end over HTTP and real BrowserOS CDP', () => {
  it('starts from sidecar config, exposes the catalog and executes browser tools', async () => {
    await withE2e(async (baseUrl) => {
      const status = await (
        await fetch(`${baseUrl}/status`, { signal: AbortSignal.timeout(5_000) })
      ).json()
      assert.equal(status.cdpConnected, true)
      const client = new Client({ name: 'server-e2e', version: '1.0.0' })
      const transport = new StreamableHTTPClientTransport(
        new URL(`${baseUrl}/mcp?structured=1`),
      )
      try {
        await client.connect(transport, { timeout: 5_000 })
        const { tools } = await client.listTools(undefined, { timeout: 5_000 })
        assert.deepEqual(tools.map((tool) => tool.name).sort(), [
          'act',
          'connector_mcp_servers',
          'diff',
          'download',
          'evaluate',
          'grep',
          'history',
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
        ])
        for (const tool of tools) assert.equal(tool.inputSchema.type, 'object')
        let session: string | undefined
        const tabs = async (args: Record<string, unknown>) => {
          const result = CallToolResultSchema.parse(
            await client.callTool(
              {
                name: 'tabs',
                arguments: { ...args, ...(session ? { session } : {}) },
              },
              undefined,
              { timeout: 5_000 },
            ),
          )
          assert.notEqual(result.isError, true, JSON.stringify(result))
          const handle = result._meta?.['com.browseros/session']
          assert.equal(typeof handle, 'string')
          if (session) assert.equal(handle, session)
          session = handle as string
          return result
        }
        const listPages = async () => {
          const result = await tabs({ action: 'list' })
          assert.notEqual(result.isError, true, JSON.stringify(result))
          const pages = result.structuredContent?.pages
          assert.ok(Array.isArray(pages), JSON.stringify(result))
          return pages as Array<{ page: number; url: string }>
        }
        assert.ok(
          (await listPages()).some((page) => page.url === 'about:blank'),
        )
        const url = `data:text/html,server-e2e-${crypto.randomUUID()}`
        const opened = await tabs({ action: 'new', url })
        assert.notEqual(opened.isError, true, JSON.stringify(opened))
        const page = opened.structuredContent?.page
        assert.equal(typeof page, 'number', JSON.stringify(opened))
        // Repeated calls use the same native HTTP client and actual custom CDP.
        for (let i = 0; i < 2; i++) {
          assert.ok(
            (await listPages()).some(
              (item) => item.page === page && item.url === url,
            ),
          )
        }
        const closed = await tabs({ action: 'close', page })
        assert.notEqual(closed.isError, true, JSON.stringify(closed))
        assert.ok(!(await listPages()).some((item) => item.page === page))
        const forbidden = await fetch(`${baseUrl}/mcp`, {
          method: 'POST',
          signal: AbortSignal.timeout(5_000),
          headers: {
            'Sec-Fetch-Site': 'cross-site',
            'Content-Type': 'application/json',
          },
          body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/list' }),
        })
        assert.equal(forbidden.status, 403)
      } finally {
        await client.close()
      }
    })
  }, 120_000)
})
