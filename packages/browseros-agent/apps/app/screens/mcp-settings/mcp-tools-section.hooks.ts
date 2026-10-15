import type { McpTool } from '@browseros/server-api'
import { useQuery } from '@tanstack/react-query'
import { serverApi, serverData } from '@/lib/server-api/client'
import { useAgentServerUrl } from '@/modules/browseros/agent-server-url.hooks'

const TOOLS_QUERY_KEY = ['mcp-manager', 'tools'] as const

async function fetchTools(agentServerUrl: string): Promise<McpTool[]> {
  const body = await serverData(
    serverApi(agentServerUrl).http.GET('/mcp-manager/tools'),
    (error, response) =>
      `Failed to list tools: ${error.status} ${response?.statusText ?? ''}`,
  )
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
