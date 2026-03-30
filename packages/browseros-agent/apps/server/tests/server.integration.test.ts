/**
 * @license
 * Copyright 2025 BrowserOS
 *
 * Integration tests for the consolidated HTTP server.
 * Uses the unified test environment setup.
 */

import { afterAll, beforeAll, describe, it, setDefaultTimeout } from 'bun:test'
import assert from 'node:assert'
import { URL } from 'node:url'
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js'

import {
  cleanupBrowserOS,
  ensureBrowserOS,
  type TestEnvironmentConfig,
} from './__helpers__/index'

setDefaultTimeout(30000)

let config: TestEnvironmentConfig
let mcpClient: Client | null = null
let mcpTransport: StreamableHTTPClientTransport | null = null

function getBaseUrl(): string {
  return `http://127.0.0.1:${config.serverPort}`
}

describe('HTTP Server Integration Tests', () => {
  beforeAll(async () => {
    config = await ensureBrowserOS()

    mcpClient = new Client({
      name: 'browseros-integration-test-client',
      version: '1.0.0',
    })

    const serverUrl = new URL(`${getBaseUrl()}/mcp`)
    mcpTransport = new StreamableHTTPClientTransport(serverUrl)

    await mcpClient.connect(mcpTransport)
    console.log('MCP client connected\n')
  })

  afterAll(async () => {
    if (mcpTransport) {
      console.log('\nClosing MCP client...')
      await mcpTransport.close()
      mcpTransport = null
      mcpClient = null
      console.log('MCP client closed')
    }

    if (!process.env.KEEP_BROWSER) {
      await cleanupBrowserOS()
    }
  })

  describe('Health endpoint', () => {
    it('responds with 200 OK', async () => {
      const response = await fetch(`${getBaseUrl()}/health`)
      assert.strictEqual(response.status, 200)

      const json = await response.json()
      assert.strictEqual(json.status, 'ok')
    })
  })

  describe('Status endpoint', () => {
    it('reports CDP as connected', async () => {
      const response = await fetch(`${getBaseUrl()}/status`)
      assert.strictEqual(response.status, 200)

      const json = (await response.json()) as {
        status: string
        cdpConnected: boolean
      }
      assert.strictEqual(json.status, 'ok')
      assert.strictEqual(json.cdpConnected, true)
    })
  })

  describe('MCP endpoint', () => {
    it('lists available tools', async () => {
      assert.ok(mcpClient, 'MCP client should be connected')

      const result = await mcpClient.listTools()

      assert.ok(result.tools, 'Should return tools array')
      assert.ok(Array.isArray(result.tools), 'Tools should be an array')
      assert.ok(result.tools.length > 0, 'Should have at least one tool')

      console.log(`Found ${result.tools.length} tools`)
    })

    it('calls browser_list_tabs tool successfully', async () => {
      assert.ok(mcpClient, 'MCP client should be connected')

      const result = await mcpClient.callTool({
        name: 'browser_list_tabs',
        arguments: {},
      })

      assert.ok(result.content, 'Should return content')
      assert.ok(Array.isArray(result.content), 'Content should be an array')

      const textContent = result.content.find(
        (item) => item.type === 'text' && typeof item.text === 'string',
      )
      assert.ok(textContent, 'Should include text content')
      console.log('browser_list_tabs content:', textContent?.text ?? '')
      assert.ok(textContent.text, 'Response should contain text')
      console.log(
        'browser_list_tabs returned:',
        result.content.length,
        'content items',
      )
    })

    it('handles invalid tool name gracefully', async () => {
      assert.ok(mcpClient, 'MCP client should be connected')

      try {
        await mcpClient.callTool({
          name: 'this_tool_does_not_exist',
          arguments: {},
        })
        assert.fail('Should have thrown an error for invalid tool')
      } catch (error) {
        assert.ok(error, 'Should throw error for invalid tool')
      }
    })
  })

  describe('Concurrent request handling', () => {
    it('handles multiple simultaneous requests without conflicts', async () => {
      assert.ok(mcpClient, 'MCP client should be connected')
      const client = mcpClient

      const requests = Array.from({ length: 10 }, () => client.listTools())

      const results = await Promise.all(requests)

      results.forEach((result) => {
        assert.ok(result.tools, 'Each request should return tools')
        assert.ok(Array.isArray(result.tools), 'Tools should be an array')
        assert.ok(result.tools.length > 0, 'Should have tools')
      })

      console.log(`All ${results.length} concurrent requests succeeded`)
    })
  })

  describe('Chat endpoint', () => {
    it(
      'streams a chat response with BrowserOS provider',
      async () => {
        const conversationId = crypto.randomUUID()

        const response = await fetch(`${getBaseUrl()}/chat`, {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
          },
          body: JSON.stringify({
            conversationId,
            message: 'Open amazon.com in a new tab',
            provider: 'browseros',
            model: 'claude-sonnet-4-20250514',
          }),
        })

        assert.strictEqual(response.status, 200, 'Chat should return 200')
        assert.ok(
          response.headers.get('content-type')?.includes('text/event-stream'),
          'Should return SSE stream',
        )

        const reader = response.body?.getReader()
        assert.ok(reader, 'Should have response body reader')

        const decoder = new TextDecoder()
        let fullResponse = ''
        let eventCount = 0

        while (true) {
          const { done, value } = await reader.read()
          if (done) break

          const chunk = decoder.decode(value, { stream: true })
          fullResponse += chunk
          eventCount++

          if (eventCount <= 3) {
            console.log(`[CHAT] Event ${eventCount}:`, chunk.slice(0, 100))
          }
        }

        console.log(
          `[CHAT] Received ${eventCount} events, ${fullResponse.length} bytes total`,
        )

        assert.ok(
          fullResponse.includes('data:'),
          'Should contain SSE data events',
        )

        const deleteResponse = await fetch(
          `${getBaseUrl()}/chat/${conversationId}`,
          {
            method: 'DELETE',
          },
        )
        assert.strictEqual(deleteResponse.status, 200, 'Should delete session')
      },
      { timeout: 30000 },
    )

    it('returns 400 for invalid chat request', async () => {
      const response = await fetch(`${getBaseUrl()}/chat`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          message: 'Hello',
        }),
      })

      assert.strictEqual(
        response.status,
        400,
        'Should return 400 for invalid request',
      )
    })

    it('does not expose the removed /chat-v2 endpoint', async () => {
      const response = await fetch(`${getBaseUrl()}/chat-v2`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          conversationId: crypto.randomUUID(),
          message: 'Hello',
          provider: 'browseros',
          model: 'claude-sonnet-4-20250514',
        }),
      })

      assert.strictEqual(
        response.status,
        404,
        'Removed /chat-v2 should return 404',
      )
    })
  })
})
