import { createMCPClient } from '@ai-sdk/mcp'
import { TIMEOUTS } from '@browseros/shared/constants/timeouts'
import type { BrowserContext } from '@browseros/shared/schemas/browser-context'
import type { ToolSet } from 'ai'
import { logger } from '../lib/logger'
import {
  detectMcpTransport,
  type McpTransportType,
} from '../lib/mcp-transport-detect'

export interface McpServerSpec {
  name: string
  url: string
  transport: McpTransportType
  headers?: Record<string, string>
}

export interface McpServerSpecDeps {
  browserContext?: BrowserContext
}

export interface McpClientBundle {
  clients: Array<{ close(): Promise<void> }>
  tools: ToolSet
}

function summarizeMcpUrl(value: string): string {
  try {
    const url = new URL(value)
    return `${url.origin}${url.pathname}`
  } catch {
    return '(invalid url)'
  }
}

/** Builds custom MCP server specs from browser context. */
export async function buildMcpServerSpecs(
  deps: McpServerSpecDeps,
): Promise<McpServerSpec[]> {
  const specs: McpServerSpec[] = []

  if (deps.browserContext?.customMcpServers?.length) {
    const servers = deps.browserContext.customMcpServers
    logger.debug('Resolving custom MCP server transports', {
      count: servers.length,
      serverNames: servers.map((server) => server.name),
    })
    const transports = await Promise.all(
      servers.map((s) => detectMcpTransport(s.url)),
    )
    for (let i = 0; i < servers.length; i++) {
      specs.push({
        name: `custom-${servers[i].name}`,
        url: servers[i].url,
        transport: transports[i],
      })
    }
    logger.debug('Custom MCP server specs resolved', {
      count: specs.length,
      specs: specs.map((spec) => ({
        name: spec.name,
        url: summarizeMcpUrl(spec.url),
        transport: spec.transport,
      })),
    })
  }

  return specs
}

async function connectMcpClient(
  spec: McpServerSpec,
): Promise<{ client: { close(): Promise<void> }; tools: ToolSet } | null> {
  const timeout = TIMEOUTS.MCP_CLIENT_CONNECT
  const logContext = {
    name: spec.name,
    url: summarizeMcpUrl(spec.url),
    transport: spec.transport,
    timeoutMs: timeout,
  }
  logger.debug('Connecting MCP client', logContext)
  try {
    const client = await Promise.race([
      createMCPClient({
        transport: {
          type: spec.transport === 'sse' ? 'sse' : 'http',
          url: spec.url,
          headers: spec.headers,
        },
      }),
      new Promise<never>((_, reject) =>
        setTimeout(
          () =>
            reject(
              new Error(`MCP client connect timed out after ${timeout}ms`),
            ),
          timeout,
        ),
      ),
    ])
    const clientTools = await Promise.race([
      client.tools(),
      new Promise<never>((_, reject) =>
        setTimeout(
          () =>
            reject(
              new Error(`MCP client.tools() timed out after ${timeout}ms`),
            ),
          timeout,
        ),
      ),
    ])
    logger.debug('MCP client connected', {
      ...logContext,
      toolCount: Object.keys(clientTools).length,
    })
    // Cast keeps the call green when this package compiles in a
    // workspace that also has zod v4 present. The two zod majors export
    // compatible runtime values but TypeScript's inferred type for
    // `client.tools()` widens from `ZodType<never>` to
    // `ZodType<unknown>` in that resolution context, which the AI
    // SDK's strict `ToolSet` rejects. The cast is shape-correct;
    // `clientTools` IS a `ToolSet` at runtime.
    return { client, tools: clientTools as ToolSet }
  } catch (error) {
    logger.warn('Failed to connect MCP client, skipping', {
      ...logContext,
      error: error instanceof Error ? error.message : String(error),
    })
    return null
  }
}

/** Connects custom MCP servers and returns their merged toolset. */
export async function createMcpClients(
  specs: McpServerSpec[],
): Promise<McpClientBundle> {
  const clients: Array<{ close(): Promise<void> }> = []
  let tools: ToolSet = {}

  if (specs.length > 0) {
    logger.debug('Creating MCP clients', {
      count: specs.length,
      serverNames: specs.map((spec) => spec.name),
    })
  }
  const results = await Promise.all(specs.map(connectMcpClient))
  for (const result of results) {
    if (result) {
      clients.push(result.client)
      tools = { ...tools, ...result.tools }
    }
  }
  if (specs.length > 0) {
    logger.debug('MCP clients created', {
      requestedCount: specs.length,
      connectedCount: clients.length,
      toolCount: Object.keys(tools).length,
    })
  }

  return { clients, tools }
}
