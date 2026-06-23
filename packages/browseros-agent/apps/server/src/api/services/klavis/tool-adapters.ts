/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { JSONValue } from '@ai-sdk/provider'
import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import type { CallToolResult } from '@modelcontextprotocol/sdk/types.js'
import type { ToolSet } from 'ai'
import { z } from 'zod'
import { logger } from '../../../lib/logger'
import { metrics } from '../../../lib/metrics'
import { findConnector, getConnectorCatalogDescription } from './catalog'
import { selectedServerNames } from './connector-state'
import type {
  ConnectorCatalogItem,
  ConnectorConnectionIntent,
  ConnectorInventory,
  ConnectorToolScope,
  KlavisProxyStatus,
  KlavisSessionHandle,
  UserIntegration,
} from './types'

export interface KlavisToolAdapterDeps {
  catalog: readonly ConnectorCatalogItem[]
  proxyStatus: KlavisProxyStatus
  session: KlavisSessionHandle | null
  scope?: ConnectorToolScope
  createConnectionIntent: (
    serverName: string,
  ) => Promise<ConnectorConnectionIntent>
  getConnectorInventory: (
    scope?: ConnectorToolScope,
  ) => Promise<ConnectorInventory>
  getUserIntegrations: () => Promise<UserIntegration[]>
}

type ConnectorToolPayload =
  | ConnectorInventory
  | {
      connected: boolean
      server_name: string
      authUrl?: string | null
      proxy: KlavisProxyStatus
      message?: string
    }

function connectorInputSchema(catalog: readonly ConnectorCatalogItem[]) {
  const names = catalog.map((server) => server.name) as [string, ...string[]]
  const serverDescriptions = getConnectorCatalogDescription()
  return {
    server_name: z
      .enum(names)
      .optional()
      .describe(
        `The optional service to check. Omit to list connector inventory. Available: ${serverDescriptions}`,
      ),
  } as unknown as Record<string, never>
}

function isObjectRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

async function buildConnectorToolPayload(
  deps: KlavisToolAdapterDeps,
  args: Record<string, unknown>,
): Promise<ConnectorToolPayload> {
  const serverName =
    typeof args.server_name === 'string' ? args.server_name : undefined
  if (!serverName) {
    return deps.getConnectorInventory(deps.scope)
  }

  const connector = findConnector(serverName)
  if (!connector) {
    throw new Error(`Invalid server: ${serverName}`)
  }

  const integrations = await deps.getUserIntegrations()
  const integration = integrations.find((item) => item.name === serverName)
  const isConnected = integration?.isAuthenticated === true
  if (isConnected) {
    return {
      connected: true,
      server_name: serverName,
      proxy: deps.proxyStatus,
    }
  }

  const intent = await deps.createConnectionIntent(serverName)
  const authUrl = intent.oauthUrl ?? intent.apiKeyUrl ?? null
  return {
    connected: false,
    server_name: serverName,
    authUrl,
    proxy: deps.proxyStatus,
    message: authUrl
      ? `${serverName} is not connected. Ask the user to open this URL to authenticate: ${authUrl}`
      : `${serverName} is not connected. Could not retrieve auth URL.`,
  }
}

function klavisResultToModelOutput(output: unknown) {
  if (!isObjectRecord(output) || !Array.isArray(output.content)) {
    return {
      type: 'json' as const,
      value: (output as JSONValue | undefined) ?? null,
    }
  }

  const result = output as CallToolResult

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

export function buildKlavisToolSet(deps: KlavisToolAdapterDeps): ToolSet {
  const toolSet: ToolSet = {
    connector_mcp_servers: {
      description:
        'Check or list BrowserOS managed app connectors before using Strata MCP tools. Omit server_name to see available, selected, connected, and proxy status. With server_name, returns connected/auth URL status.',
      inputSchema: z.object(
        connectorInputSchema(deps.catalog) as z.ZodRawShape,
      ),
      execute: async (args: Record<string, unknown>) =>
        buildConnectorToolPayload(deps, args),
      toModelOutput: ({ output }: { output: unknown }) => ({
        type: 'text' as const,
        value: JSON.stringify(output),
      }),
    } satisfies ToolSet[string],
  }

  const session = deps.session
  if (deps.proxyStatus.state !== 'ready' || !session) {
    return toolSet
  }

  for (const t of session.tools) {
    const rawShape = session.inputSchemas.get(t.name)
    const name = t.name
    toolSet[name] = {
      description: t.description ?? '',
      inputSchema: z.object((rawShape ?? {}) as z.ZodRawShape),
      execute: async (args: Record<string, unknown>) =>
        session.callTool(name, args),
      toModelOutput: ({ output }: { output: unknown }) =>
        klavisResultToModelOutput(output),
    } satisfies ToolSet[string]
  }

  return toolSet
}

export function registerKlavisTools(
  mcpServer: McpServer,
  deps: KlavisToolAdapterDeps,
): void {
  mcpServer.registerTool(
    'connector_mcp_servers',
    {
      description:
        'Check or list BrowserOS managed app connectors before using Strata MCP tools. Omit server_name to see available, selected, connected, and proxy status. With server_name, returns connected/auth URL status.',
      inputSchema: connectorInputSchema(deps.catalog),
    },
    async (args: Record<string, unknown>) => {
      const startTime = performance.now()

      try {
        const payload = await buildConnectorToolPayload(deps, args)

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
              text: JSON.stringify(payload),
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

  const session = deps.session
  if (deps.proxyStatus.state !== 'ready' || !session) {
    logger.debug('Registered Klavis connector discovery on MCP server', {
      proxyState: deps.proxyStatus.state,
      selectedServers: selectedServerNames(deps.scope),
    })
    return
  }

  for (const strataTool of session.tools) {
    const inputSchema = session.inputSchemas.get(strataTool.name)

    mcpServer.registerTool(
      strataTool.name,
      {
        description: strataTool.description,
        inputSchema,
      },
      async (args: Record<string, unknown>) => {
        const startTime = performance.now()
        try {
          const result = await session.callTool(strataTool.name, args)

          metrics.log('tool_executed', {
            tool_name: strataTool.name,
            source: 'mcp',
            duration_ms: Math.round(performance.now() - startTime),
            success: !result?.isError,
          })

          return result
        } catch (error) {
          const errorText =
            error instanceof Error ? error.message : String(error)

          metrics.log('tool_executed', {
            tool_name: strataTool.name,
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
    count: session.tools.length + 1,
    selectedServers: selectedServerNames(deps.scope),
  })
}
