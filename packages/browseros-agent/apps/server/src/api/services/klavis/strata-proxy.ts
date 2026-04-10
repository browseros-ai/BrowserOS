/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { TIMEOUTS } from '@browseros/shared/constants/timeouts'
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js'
import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import type { CallToolResult, Tool } from '@modelcontextprotocol/sdk/types.js'
import { jsonSchemaObjectToZodRawShape } from 'zod-from-json-schema'
import type { KlavisClient } from '../../../lib/clients/klavis/klavis-client'
import { OAUTH_MCP_SERVERS } from '../../../lib/clients/klavis/oauth-mcp-servers'
import { logger } from '../../../lib/logger'
import { metrics } from '../../../lib/metrics'
import { klavisStrataCache } from './strata-cache'

function withTimeout<T>(promise: Promise<T>, label: string): Promise<T> {
  let timerId: ReturnType<typeof setTimeout> | undefined
  const timeout = new Promise<T>((_, reject) => {
    timerId = setTimeout(
      () => reject(new Error(`Klavis ${label} timed out`)),
      TIMEOUTS.KLAVIS_FETCH,
    )
  })
  return Promise.race([promise, timeout]).finally(() => clearTimeout(timerId))
}

export interface KlavisProxyHandle {
  tools: Tool[]
  inputSchemas: Map<string, Record<string, never>>
  callTool: (
    name: string,
    args: Record<string, unknown>,
  ) => Promise<CallToolResult>
  close: () => Promise<void>
}

interface ConnectDeps {
  klavisClient: KlavisClient
  browserosId: string
}

// One-time async setup: connect to Klavis Strata and discover tools
export async function connectKlavisProxy(
  deps: ConnectDeps,
): Promise<KlavisProxyHandle> {
  // Use the full curated OAuth server list so all tools are exposed,
  // even unauthenticated ones (Klavis handles auth prompts on call)
  const allServers = OAUTH_MCP_SERVERS.map((s) => s.name)

  const strata = await klavisStrataCache.getOrFetch(
    deps.klavisClient,
    deps.browserosId,
    allServers,
  )

  // Connect MCP client to Strata endpoint
  const client = new Client({
    name: 'browseros-klavis-proxy',
    version: '1.0.0',
  })
  const transport = new StreamableHTTPClientTransport(
    new URL(strata.strataServerUrl),
  )
  await withTimeout(client.connect(transport), 'connect')

  const { tools } = await withTimeout(client.listTools(), 'listTools')

  // Pre-compute Zod schemas once so registerKlavisTools avoids per-request conversion.
  // Double cast works around TS2589 in registerTool's recursive generics.
  const inputSchemas = new Map(
    tools.map((t) => [
      t.name,
      jsonSchemaObjectToZodRawShape(
        t.inputSchema as never,
      ) as unknown as Record<string, never>,
    ]),
  )

  logger.info('Klavis proxy connected', {
    toolCount: tools.length,
    serverCount: allServers.length,
  })

  return {
    tools,
    inputSchemas,
    callTool: (name, args) =>
      client.callTool({ name, arguments: args }) as Promise<CallToolResult>,
    close: () => client.close(),
  }
}

export function registerKlavisTools(
  mcpServer: McpServer,
  handle: KlavisProxyHandle,
): void {
  for (const tool of handle.tools) {
    const inputSchema = handle.inputSchemas.get(tool.name)

    mcpServer.registerTool(
      tool.name,
      {
        description: tool.description,
        inputSchema,
      },
      async (args: Record<string, unknown>) => {
        const startTime = performance.now()
        try {
          const result = await handle.callTool(tool.name, args)

          metrics.log('tool_executed', {
            tool_name: tool.name,
            source: 'mcp',
            duration_ms: Math.round(performance.now() - startTime),
            success: !result.isError,
          })

          return result
        } catch (error) {
          const errorText =
            error instanceof Error ? error.message : String(error)

          metrics.log('tool_executed', {
            tool_name: tool.name,
            source: 'mcp',
            duration_ms: Math.round(performance.now() - startTime),
            success: false,
            error_message: errorText,
          })

          return {
            content: [{ type: 'text' as const, text: errorText }],
            isError: true,
          }
        }
      },
    )
  }

  logger.debug('Registered Klavis tools on MCP server', {
    count: handle.tools.length,
  })
}
