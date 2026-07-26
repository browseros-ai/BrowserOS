/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, it } from 'bun:test'
import assert from 'node:assert'
import fs from 'node:fs'
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js'
import { executablePath } from 'puppeteer'

// TODO: Re-enable after Phase 4 (HTTP Server) implementation
// This test uses old CLI args (--headless, --isolated, --executable-path)
// which were removed in Phase 1. Need to update for new architecture:
// - Start browser with CDP on specific port
// - Start MCP server with sidecar --config
// - Test via HTTP/SSE transport instead of STDIO
describe.skip('e2e', () => {
  async function withClient(cb: (client: Client) => Promise<void>) {
    const transport = new StdioClientTransport({
      command: 'node',
      args: [
        'build/src/index.js',
        '--headless',
        '--isolated',
        '--executable-path',
        executablePath(),
      ],
    })
    const client = new Client(
      {
        name: 'e2e-test',
        version: '1.0.0',
      },
      {
        capabilities: {},
      },
    )

    try {
      await client.connect(transport)
      await cb(client)
    } finally {
      await client.close()
    }
  }
  it('calls a tool', async () => {
    await withClient(async (client) => {
      const result = await client.callTool({
        name: 'list_pages',
        arguments: {},
      })
      assert.deepStrictEqual(result, {
        content: [
          {
            type: 'text',
            text: '# list_pages response\n## Pages\n0: about:blank [selected]',
          },
        ],
      })
    })
  })

  it('calls a tool multiple times', async () => {
    await withClient(async (client) => {
      let result = await client.callTool({
        name: 'list_pages',
        arguments: {},
      })
      result = await client.callTool({
        name: 'list_pages',
        arguments: {},
      })
      assert.deepStrictEqual(result, {
        content: [
          {
            type: 'text',
            text: '# list_pages response\n## Pages\n0: about:blank [selected]',
          },
        ],
      })
    })
  })

  it('has all tools', async () => {
    await withClient(async (client) => {
      const { tools } = await client.listTools()
      const exposedNames = tools.map((t) => t.name).sort()
      const files = fs.readdirSync('build/src/tools')
      const definedNames = []
      for (const file of files) {
        if (file === 'ToolDefinition.js') {
          continue
        }
        const fileTools = await import(`../src/tools/${file}`)
        for (const maybeTool of Object.values<object>(fileTools)) {
          if ('name' in maybeTool) {
            definedNames.push(maybeTool.name)
          }
        }
      }
      definedNames.sort()
      assert.deepStrictEqual(exposedNames, definedNames)
    })
  })
})
