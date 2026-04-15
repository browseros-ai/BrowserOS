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

// Build list of custom MCP server specs from browser context
// (Klavis Strata is handled separately via shared background connection)
export async function buildMcpServerSpecs(
  deps: McpServerSpecDeps,
): Promise<McpServerSpec[]> {
  const specs: McpServerSpec[] = []

  // User-provided custom MCP servers
  if (deps.browserContext?.customMcpServers?.length) {
    const servers = deps.browserContext.customMcpServers
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
  }

  return specs
}

// Connect a single MCP client with timeout protection
async function connectMcpClient(
  spec: McpServerSpec,
): Promise<{ client: { close(): Promise<void> }; tools: ToolSet } | null> {
  const timeout = TIMEOUTS.MCP_CLIENT_CONNECT
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
    return { client, tools: clientTools }
  } catch (error) {
    logger.warn('Failed to connect MCP client, skipping', {
      name: spec.name,
      url: spec.url,
      error: error instanceof Error ? error.message : String(error),
    })
    return null
  }
}

// Create MCP clients from specs, return merged toolset
export async function createMcpClients(
  specs: McpServerSpec[],
): Promise<McpClientBundle> {
  const clients: Array<{ close(): Promise<void> }> = []
  let tools: ToolSet = {}

  // Connect all clients concurrently with per-client timeout
  const results = await Promise.all(specs.map(connectMcpClient))
  for (const result of results) {
    if (result) {
      clients.push(result.client)
      tools = { ...tools, ...result.tools }
    }
  }

  return { clients, tools }
}
