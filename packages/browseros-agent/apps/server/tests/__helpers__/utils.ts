import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js'
import type { CallToolResult } from '@modelcontextprotocol/sdk/types.js'
import { Mutex } from 'async-mutex'

import { ensureBrowserOS } from './setup'

// =============================================================================
// Test Wrappers
// =============================================================================

const envMutex = new Mutex()

export async function withMcpServer(
  cb: (client: Client) => Promise<void>,
): Promise<void> {
  return await envMutex.runExclusive(async () => {
    const config = await ensureBrowserOS()

    const client = new Client({
      name: 'browseros-test-client',
      version: '1.0.0',
    })

    const serverUrl = new URL(`http://127.0.0.1:${config.serverPort}/mcp`)
    const transport = new StreamableHTTPClientTransport(serverUrl)

    try {
      await client.connect(transport)
      await cb(client)
    } finally {
      await transport.close()
    }
  })
}

// =============================================================================
// HTML Helper
// =============================================================================

export function html(
  strings: TemplateStringsArray,
  ...values: unknown[]
): string {
  const bodyContent = strings.reduce((acc, str, i) => {
    return acc + str + (values[i] || '')
  }, '')

  return `<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>My test page</title>
  </head>
  <body>
    ${bodyContent}
  </body>
</html>`
}

// =============================================================================
// Type Helpers
// =============================================================================

export interface McpContentItem {
  type: 'text' | 'image'
  text?: string
  data?: string
  mimeType?: string
}

export interface TypedCallToolResult {
  content: McpContentItem[]
  isError?: boolean
}

export function asToolResult(result: CallToolResult): TypedCallToolResult {
  return result as unknown as TypedCallToolResult
}
