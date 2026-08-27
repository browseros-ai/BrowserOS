import type { BrowserSession } from '@browseros/browser-core/core/session'
import {
  McpServer,
  SUPPORTED_PROTOCOL_VERSIONS,
} from '@modelcontextprotocol/server'
import { BROWSER_MCP_INSTRUCTIONS } from './mcp-prompt'
import {
  type BrowserToolDefaults,
  type BrowserToolRegistrationOptions,
  registerBrowserTools,
} from './register'

// The SDK's default supportedProtocolVersions is legacy-only, so it never
// registers server/discover and rejects 2026-07-28. Advertise the modern
// revision (selectable only via server/discover) alongside the legacy list so
// the endpoint serves both eras. The SDK does not export a modern-versions list.
const MODERN_PROTOCOL_VERSION = '2026-07-28'

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
      instructions: options.instructions ?? BROWSER_MCP_INSTRUCTIONS,
      supportedProtocolVersions: [
        MODERN_PROTOCOL_VERSION,
        ...SUPPORTED_PROTOCOL_VERSIONS,
      ],
    },
  )

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
