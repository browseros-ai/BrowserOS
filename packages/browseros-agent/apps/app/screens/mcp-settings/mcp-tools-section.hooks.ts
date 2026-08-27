import { useQuery } from '@tanstack/react-query'
import type { McpTool } from '@/lib/mcp/client'
import { useAgentServerUrl } from '@/modules/browseros/agent-server-url.hooks'

interface ListToolsResponse {
  tools: McpTool[]
}

const TOOLS_QUERY_KEY = ['mcp-manager', 'tools'] as const

async function fetchTools(agentServerUrl: string): Promise<McpTool[]> {
  const res = await fetch(`${agentServerUrl}/mcp-manager/tools`)
  if (!res.ok) {
    throw new Error(`Failed to list tools: ${res.status} ${res.statusText}`)
  }
  const body = (await res.json()) as ListToolsResponse
  return body.tools
}

/**
 * Returns the BrowserOS MCP tool catalogue for the settings UI. Reads the
 * read-only `/mcp-manager/tools` endpoint rather than speaking MCP to `/mcp`,
 * which the browser cannot reach directly.
 */
export function useMcpTools() {
  const { baseUrl } = useAgentServerUrl()
  return useQuery({
    queryKey: TOOLS_QUERY_KEY,
    enabled: !!baseUrl,
    staleTime: 5_000,
    queryFn: () => {
      if (!baseUrl) throw new Error('Agent server URL is unavailable')
      return fetchTools(baseUrl)
    },
  })
}
