/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { JSONValue } from '@ai-sdk/provider'
import type { CallToolResult } from '@modelcontextprotocol/client'
import { fromJsonSchema, type McpServer } from '@modelcontextprotocol/server'
import { jsonSchema, type ToolSet } from 'ai'
import { z } from 'zod'
import { zodToJsonSchema } from 'zod-to-json-schema'
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

export interface KlavisMcpRegistrationOptions {
  /** Returns a user-facing rejection when the current MCP caller lacks authority. */
  authorizeCall?: () => string | undefined
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

function summarizeKlavisArgs(args: Record<string, unknown>) {
  const summary: Record<string, unknown> = {
    argKeys: Object.keys(args).sort(),
  }
  if (typeof args.server_name === 'string') {
    summary.serverName = args.server_name
  }
  return summary
}

function summarizeConnectorPayload(payload: ConnectorToolPayload) {
  if ('available' in payload) {
    return {
      type: 'inventory',
      availableCount: payload.available.length,
      connectedServers: payload.connected.map((item) => item.name),
      selectedServers: payload.selected,
      proxyState: payload.proxy.state,
    }
  }
  return {
    type: 'connection',
    serverName: payload.server_name,
    connected: payload.connected,
    hasAuthUrl: Boolean(payload.authUrl),
    proxyState: payload.proxy.state,
  }
}

function summarizeCallToolError(
  result: unknown,
): Record<string, unknown> | undefined {
  if (!isObjectRecord(result) || !Array.isArray(result.content)) {
    return undefined
  }
  const textBlocks = result.content
    .filter(
      (item): item is { type: 'text'; text: string } =>
        typeof item === 'object' &&
        item !== null &&
        'type' in item &&
        item.type === 'text' &&
        'text' in item &&
        typeof item.text === 'string',
    )
    .map((item) => item.text)
  const text = textBlocks.join('\n')
  return {
    contentCount: result.content.length,
    textBlockCount: textBlocks.length,
    textLength: text.length,
    lineCount: text.length ? text.split('\n').length : 0,
  }
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
    const name = t.name
    toolSet[name] = {
      description: t.description ?? '',
      // Pass the remote JSON schema straight through. Round-tripping it through
      // Zod (zod-from-json-schema -> zod-to-json-schema under zod v3) silently
      // drops every property, leaving the connector's tools unusable.
      inputSchema: jsonSchema(
        t.inputSchema as Parameters<typeof jsonSchema>[0],
      ),
      execute: async (args: Record<string, unknown>) =>
        session.callTool(name, args),
      toModelOutput: ({ output }: { output: unknown }) =>
        klavisResultToModelOutput(output),
    } satisfies ToolSet[string]
  }

  return toolSet
}

// Bridge a locally hand-built Zod v3 shape (the connector tool's schema) into a
// v2 registerTool input schema. v2 derives a tools/list JSON schema only from
// Zod v4, so convert the v3 shape to JSON Schema first, then wrap it. Remote
// Strata tools do NOT use this path: their JSON schema goes straight to
// fromJsonSchema (round-tripping through Zod drops every property under v3).
function toV2InputSchema(rawShape: z.ZodRawShape) {
  // The bridged value is a StandardSchemaWithJSON; type it as a raw shape so
  // registerTool's overload and handler typing resolve as before. At runtime
  // v2 accepts the real object via the Standard Schema path.
  return fromJsonSchema(
    zodToJsonSchema(z.object(rawShape)) as never,
  ) as unknown as Record<string, never>
}

// Wrap a remote Strata tool's JSON schema for v2 registerTool without any Zod
// round-trip. Same cast rationale as toV2InputSchema: the typed overload wants a
// raw shape, while the StandardSchemaWithJSON flows through at runtime.
function remoteJsonInputSchema(schema: Parameters<typeof fromJsonSchema>[0]) {
  return fromJsonSchema(schema) as unknown as Record<string, never>
}

export function registerKlavisTools(
  mcpServer: McpServer,
  deps: KlavisToolAdapterDeps,
  options: KlavisMcpRegistrationOptions = {},
): void {
  mcpServer.registerTool(
    'connector_mcp_servers',
    {
      description:
        'Check or list BrowserOS managed app connectors before using Strata MCP tools. Omit server_name to see available, selected, connected, and proxy status. With server_name, returns connected/auth URL status.',
      inputSchema: toV2InputSchema(
        connectorInputSchema(deps.catalog) as z.ZodRawShape,
      ),
    },
    async (args: Record<string, unknown>) => {
      const rejection = rejectUnauthorizedMcpCall(options)
      if (rejection) return rejection

      const startTime = performance.now()
      const selectedServers = selectedServerNames(deps.scope)
      const logBase = {
        toolName: 'connector_mcp_servers',
        source: 'mcp',
        proxyState: deps.proxyStatus.state,
        selectedServers,
      }
      logger.debug('MCP Klavis connector tool started', {
        ...logBase,
        args: summarizeKlavisArgs(args),
      })

      try {
        const payload = await buildConnectorToolPayload(deps, args)
        const durationMs = Math.round(performance.now() - startTime)

        metrics.log('tool_executed', {
          tool_name: 'connector_mcp_servers',
          source: 'mcp',
          duration_ms: durationMs,
          success: true,
        })
        logger.debug('MCP Klavis connector tool completed', {
          ...logBase,
          durationMs,
          result: summarizeConnectorPayload(payload),
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
        const durationMs = Math.round(performance.now() - startTime)

        metrics.log('tool_executed', {
          tool_name: 'connector_mcp_servers',
          source: 'mcp',
          duration_ms: durationMs,
          success: false,
          error_message: errorText,
        })
        logger.info('MCP Klavis connector tool returned error', {
          ...logBase,
          durationMs,
          error: errorText,
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
      selectedServerCount: selectedServerNames(deps.scope).length,
    })
    return
  }

  const selectedServers = selectedServerNames(deps.scope)
  for (const strataTool of session.tools) {
    mcpServer.registerTool(
      strataTool.name,
      {
        description: strataTool.description,
        // Pass the remote JSON schema straight through. The old Zod round-trip
        // dropped every property (zod v3 + zod-from-json-schema), so registerTool
        // advertised an empty schema that rejected all arguments.
        inputSchema: remoteJsonInputSchema(
          strataTool.inputSchema as Parameters<typeof fromJsonSchema>[0],
        ),
      },
      async (args: Record<string, unknown>) => {
        const rejection = rejectUnauthorizedMcpCall(options)
        if (rejection) return rejection

        const startTime = performance.now()
        const logBase = {
          toolName: strataTool.name,
          source: 'mcp',
          proxyState: deps.proxyStatus.state,
          selectedServers,
        }
        logger.debug('MCP Klavis tool started', {
          ...logBase,
          args: summarizeKlavisArgs(args),
        })
        try {
          const result = await session.callTool(strataTool.name, args)
          const durationMs = Math.round(performance.now() - startTime)

          metrics.log('tool_executed', {
            tool_name: strataTool.name,
            source: 'mcp',
            duration_ms: durationMs,
            success: !result?.isError,
          })
          logger.debug('MCP Klavis tool completed', {
            ...logBase,
            durationMs,
            isError: Boolean(result?.isError),
            contentCount: Array.isArray(result?.content)
              ? result.content.length
              : 0,
          })
          if (result?.isError) {
            logger.info('MCP Klavis tool returned error', {
              ...logBase,
              durationMs,
              errorSummary: summarizeCallToolError(result),
            })
          }

          return result
        } catch (error) {
          const errorText =
            error instanceof Error ? error.message : String(error)
          const durationMs = Math.round(performance.now() - startTime)

          metrics.log('tool_executed', {
            tool_name: strataTool.name,
            source: 'mcp',
            duration_ms: durationMs,
            success: false,
            error_message: errorText,
          })
          logger.info('MCP Klavis tool threw', {
            ...logBase,
            durationMs,
            error: errorText,
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
    selectedServers,
    selectedServerCount: selectedServers.length,
  })
}

/** Keeps connector handlers behind the same lease as browser-tool handlers. */
function rejectUnauthorizedMcpCall(options: KlavisMcpRegistrationOptions) {
  const message = options.authorizeCall?.()
  if (!message) return undefined
  return {
    content: [{ type: 'text' as const, text: message }],
    isError: true,
  }
}
