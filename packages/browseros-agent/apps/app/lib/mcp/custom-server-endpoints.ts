/**
 * Projects extension-owned MCP configuration into the server's chat context.
 * The form validates new URLs, but older storage can contain incomplete rows;
 * omit those before crossing the generated contract rather than asserting a URL.
 */
import type { CustomMcpServer } from '@browseros/server-api'
import type { McpServer } from './mcpServerStorage'

export function customServerEndpoints(
  servers: readonly Pick<McpServer, 'type' | 'displayName' | 'config'>[],
): CustomMcpServer[] {
  return servers.flatMap((server) => {
    const url = server.config?.url
    return server.type === 'custom' && url && URL.canParse(url)
      ? [{ name: server.displayName, url }]
      : []
  })
}
