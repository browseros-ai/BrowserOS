/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { JSONValue } from '@ai-sdk/provider'
import {
  KLAVIS_PROXY_RETRY_BACKOFF_MS,
  TIMEOUTS,
} from '@browseros/shared/constants/timeouts'
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js'
import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import type { CallToolResult, Tool } from '@modelcontextprotocol/sdk/types.js'
import type { ToolSet } from 'ai'
import { z } from 'zod'
import { jsonSchemaObjectToZodRawShape } from 'zod-from-json-schema'
import { KlavisClient } from '../../../lib/clients/klavis/klavis-client'
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
  browserosId: string
  tools: Tool[]
  inputSchemas: Map<string, Record<string, never>>
  callTool: (
    name: string,
    args: Record<string, unknown>,
  ) => Promise<CallToolResult>
  close: () => Promise<void>
}

export interface KlavisProxyRef {
  handle: KlavisProxyHandle | null
}

interface ConnectDeps {
  klavisClient: KlavisClient
  browserosId: string
}

interface BackgroundConnectOptions {
  connect?: (deps: ConnectDeps) => Promise<KlavisProxyHandle>
  retryDelaysMs?: readonly number[]
}

export async function connectKlavisProxy(
  deps: ConnectDeps,
): Promise<KlavisProxyHandle> {
  const allServers = OAUTH_MCP_SERVERS.map((s) => s.name)

  const strata = await klavisStrataCache.getOrFetch(
    deps.klavisClient,
    deps.browserosId,
    allServers,
  )

  const client = new Client({
    name: 'browseros-klavis-proxy',
    version: '1.0.0',
  })
  const transport = new StreamableHTTPClientTransport(
    new URL(strata.strataServerUrl),
  )
  await withTimeout(client.connect(transport), 'connect')

  const { tools } = await withTimeout(client.listTools(), 'listTools')

  const inputSchemas = new Map(
    tools.map((t) => [
      t.name,
      jsonSchemaObjectToZodRawShape(
        t.inputSchema as never,
      ) as unknown as Record<string, never>,
    ]),
  )

  return {
    browserosId: deps.browserosId,
    tools,
    inputSchemas,
    callTool: (name, args) =>
      withTimeout(
        client.callTool({ name, arguments: args }) as Promise<CallToolResult>,
        `callTool(${name})`,
      ),
    close: () => client.close(),
  }
}

const serverNames = OAUTH_MCP_SERVERS.map((s) => s.name) as [
  string,
  ...string[],
]

const serverDescriptions = OAUTH_MCP_SERVERS.map(
  (s) => `${s.name} (${s.description})`,
).join(', ')

const connectorInputSchema = {
  server_name: z
    .enum(serverNames)
    .describe(
      `The name of the service to check. Available: ${serverDescriptions}`,
    ),
} as unknown as Record<string, never>
function klavisResultToModelOutput(output: unknown) {
  const result = output as CallToolResult

  if (!('content' in result) || !Array.isArray(result.content)) {
    return {
      type: 'json' as const,
      value: (result as JSONValue | undefined) ?? null,
    }
  }

  return {
    type: 'content' as const,
    value: result.content.map((part) => {
      if (part.type === 'text') {
        return {
          type: 'text' as const,
          text: part.text,
        }
      }
      if (part.type === 'image') {
        return {
          type: 'image-data' as const,
          data: part.data,
          mediaType: part.mimeType ?? 'image/png',
        }
      }
      return {
        type: 'text' as const,
        text: JSON.stringify(part),
      }
    }),
  }
}

export function connectKlavisInBackground(
  ref: KlavisProxyRef,
  deps: ConnectDeps,
  options: BackgroundConnectOptions = {},
): () => void {
  const connect = options.connect ?? connectKlavisProxy
  const retryDelaysMs = options.retryDelaysMs ?? KLAVIS_PROXY_RETRY_BACKOFF_MS
  let stopped = false
  let retryTimer: ReturnType<typeof setTimeout> | undefined

  async function attempt(n: number): Promise<void> {
    if (stopped) return

    try {
      const handle = await connect(deps)
      if (stopped) {
        await handle.close().catch((error) => {
          logger.warn('Failed to close Klavis proxy transport after stop', {
            error: error instanceof Error ? error.message : String(error),
          })
        })
        return
      }
      ref.handle = handle
      logger.info('Klavis proxy connected', {
        attempt: n + 1,
        toolCount: handle.tools.length,
      })
    } catch (error) {
      if (stopped) return

      const msg = error instanceof Error ? error.message : String(error)
      if (n < retryDelaysMs.length) {
        const delay = retryDelaysMs[n]
        logger.info('Retrying Klavis proxy connection', {
          attempt: n + 1,
          nextRetryMs: delay,
          error: msg,
        })
        retryTimer = setTimeout(() => {
          retryTimer = undefined
          void attempt(n + 1)
        }, delay)
      } else {
        logger.warn(
          'Klavis proxy connection failed after all retries, MCP will serve browser tools only',
          { attempts: n + 1, error: msg },
        )
      }
    }
  }

  void attempt(0)

  return () => {
    stopped = true
    if (retryTimer) {
      clearTimeout(retryTimer)
      retryTimer = undefined
    }
  }
}

export function buildKlavisToolSet(handle: KlavisProxyHandle): ToolSet {
  const toolSet: ToolSet = {}

  for (const t of handle.tools) {
    const rawShape = handle.inputSchemas.get(t.name)
    const name = t.name
    toolSet[name] = {
      description: t.description ?? '',
      inputSchema: z.object((rawShape ?? {}) as z.ZodRawShape),
      execute: async (args: Record<string, unknown>) =>
        handle.callTool(name, args),
      toModelOutput: ({ output }: { output: unknown }) =>
        klavisResultToModelOutput(output),
    } satisfies ToolSet[string]
  }

  return toolSet
}

export function registerKlavisTools(
  mcpServer: McpServer,
  handle: KlavisProxyHandle,
): void {
  mcpServer.registerTool(
    'connector_mcp_servers',
    {
      description:
        'Check if an external service is connected and ready for use with Strata MCP tools (discover_server_categories_or_actions, execute_action, etc.). Call this BEFORE using any Strata integration tool. If connected, proceed with Strata tools. If not connected, returns an authUrl — prompt the user to open it and authenticate.',
      inputSchema: connectorInputSchema,
    },
    async (args: Record<string, unknown>) => {
      const startTime = performance.now()
      const server_name = args.server_name as string

      try {
        const klavisClient = new KlavisClient()
        const integrations = await klavisClient.getUserIntegrations(
          handle.browserosId,
        )

        const integration = integrations.find((i) => i.name === server_name)
        const isConnected = integration?.isAuthenticated === true

        if (isConnected) {
          metrics.log('tool_executed', {
            tool_name: 'connector_mcp_servers',
            source: 'mcp',
            duration_ms: Math.round(performance.now() - startTime),
            success: true,
          })

          return {
            content: [
              {
                type: 'text' as const,
                text: JSON.stringify({
                  connected: true,
                  server_name,
                }),
              },
            ],
          }
        }

        const strata = await klavisClient.createStrata(handle.browserosId, [
          server_name,
        ])
        const authUrl =
          strata.oauthUrls?.[server_name] ??
          strata.apiKeyUrls?.[server_name] ??
          null

        metrics.log('tool_executed', {
          tool_name: 'connector_mcp_servers',
          source: 'mcp',
          duration_ms: Math.round(performance.now() - startTime),
          success: true,
        })

        return {
          content: [
            {
              type: 'text' as const,
              text: JSON.stringify({
                connected: false,
                server_name,
                authUrl,
                message: authUrl
                  ? `${server_name} is not connected. Ask the user to open this URL to authenticate: ${authUrl}`
                  : `${server_name} is not connected. Could not retrieve auth URL.`,
              }),
            },
          ],
        }
      } catch (error) {
        const errorText = error instanceof Error ? error.message : String(error)

        metrics.log('tool_executed', {
          tool_name: 'connector_mcp_servers',
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
    count: handle.tools.length + 1,
  })
}
