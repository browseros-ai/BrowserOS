import type { BrowserSession } from '@browseros/browser-core/core/session'
import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { SetLevelRequestSchema } from '@modelcontextprotocol/sdk/types.js'
import { BROWSER_MCP_INSTRUCTIONS } from './mcp-prompt'
import {
  type BrowserToolDefaults,
  type BrowserToolRegistrationOptions,
  registerBrowserTools,
} from './register'

export interface BrowserMcpServerOptions extends BrowserToolDefaults {
  name: string
  title: string
  version: string
  browserSession: BrowserSession
  instructions?: string
  registration?: BrowserToolRegistrationOptions
}

/** Creates a BrowserOS MCP server with only the shared browser tool surface. */
export function createBrowserMcpServer(
  options: BrowserMcpServerOptions,
): McpServer {
  const server = new McpServer(
    {
      name: options.name,
      title: options.title,
      version: options.version,
    },
    {
      capabilities: { logging: {} },
      instructions: options.instructions ?? BROWSER_MCP_INSTRUCTIONS,
    },
  )

  server.server.setRequestHandler(SetLevelRequestSchema, () => {
    return {}
  })

  registerBrowserTools(
    server,
    options.browserSession,
    {
      defaultWindowId: options.defaultWindowId,
      defaultTabGroupId: options.defaultTabGroupId,
    },
    options.registration,
  )

  return server
}
