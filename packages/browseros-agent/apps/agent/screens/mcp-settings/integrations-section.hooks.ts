import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useAgentServerUrl } from '@/modules/browseros/agent-server-url.hooks'

export interface McpAgentRow {
  id: string
  displayName: string
  installed: boolean
  linked: boolean
  configPath: string | null
}

interface ListAgentsResponse {
  agents: McpAgentRow[]
}

interface MutationResponse {
  success: boolean
  message?: string
}

const AGENTS_QUERY_KEY = ['mcp-manager', 'agents'] as const

async function fetchAgents(agentServerUrl: string): Promise<McpAgentRow[]> {
  const res = await fetch(`${agentServerUrl}/mcp-manager/agents`)
  if (!res.ok) {
    throw new Error(`Failed to list agents: ${res.status} ${res.statusText}`)
  }
  const body = (await res.json()) as ListAgentsResponse
  return body.agents
}

async function callMutation(
  agentServerUrl: string,
  agentId: string,
  action: 'install' | 'uninstall',
): Promise<MutationResponse> {
  const res = await fetch(
    `${agentServerUrl}/mcp-manager/agents/${encodeURIComponent(agentId)}/${action}`,
    {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
    },
  )
  const body = (await res.json().catch(() => ({}))) as MutationResponse
  if (!res.ok && body.success !== false) {
    throw new Error(body.message ?? `Failed to ${action} agent: ${res.status}`)
  }
  return body
}

/** Returns the live agent-detection + link-state list. Polled lazily. */
export function useMcpAgents() {
  const { baseUrl } = useAgentServerUrl()
  return useQuery({
    queryKey: AGENTS_QUERY_KEY,
    enabled: !!baseUrl,
    staleTime: 5_000,
    queryFn: () => {
      if (!baseUrl) throw new Error('Agent server URL is unavailable')
      return fetchAgents(baseUrl)
    },
  })
}

/** Mutation: install BrowserOS as MCP into a single agent. */
export function useInstallAgent() {
  const { baseUrl } = useAgentServerUrl()
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (agentId: string) => {
      if (!baseUrl) throw new Error('Agent server URL is unavailable')
      return callMutation(baseUrl, agentId, 'install')
    },
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: AGENTS_QUERY_KEY })
    },
  })
}

/** Mutation: remove BrowserOS as MCP from a single agent. */
export function useUninstallAgent() {
  const { baseUrl } = useAgentServerUrl()
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (agentId: string) => {
      if (!baseUrl) throw new Error('Agent server URL is unavailable')
      return callMutation(baseUrl, agentId, 'uninstall')
    },
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: AGENTS_QUERY_KEY })
    },
  })
}
