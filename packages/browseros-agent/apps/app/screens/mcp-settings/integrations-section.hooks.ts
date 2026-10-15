import type {
  McpAgent,
  McpInstallRequest,
  McpInstallResult,
} from '@browseros/server-api'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { serverApi, serverData, serverResult } from '@/lib/server-api/client'
import { useAgentServerUrl } from '@/modules/browseros/agent-server-url.hooks'

export type McpAgentRow = McpAgent

const AGENTS_QUERY_KEY = ['mcp-manager', 'agents'] as const

async function fetchAgents(agentServerUrl: string): Promise<McpAgentRow[]> {
  const body = await serverData(
    serverApi(agentServerUrl).http.GET('/mcp-manager/agents'),
    (error, response) =>
      `Failed to list agents: ${error.status} ${response?.statusText ?? ''}`,
  )
  return body.agents
}

async function callMutation(
  agentServerUrl: string,
  agentId: string,
  action: 'install' | 'uninstall',
  payload?: McpInstallRequest,
): Promise<McpInstallResult> {
  return serverResult(
    agentServerUrl,
    (client) =>
      action === 'install'
        ? client.http.POST('/mcp-manager/agents/{id}/install', {
            params: { path: { id: agentId } },
            body: payload ?? {},
          })
        : client.http.POST('/mcp-manager/agents/{id}/uninstall', {
            params: { path: { id: agentId } },
            // Uninstall has no declared body; the old client sends an empty object.
            headers: { 'Content-Type': 'application/json' },
            middleware: [
              {
                onRequest: ({ request }) =>
                  new Request(request, { body: '{}' }),
              },
            ],
          }),
    (value): value is McpInstallResult =>
      typeof value === 'object' &&
      value !== null &&
      'success' in value &&
      value.success === false,
    async (error, response) => {
      const body: unknown = await response
        ?.clone()
        .json()
        .catch(() => null)
      return body &&
        typeof body === 'object' &&
        'message' in body &&
        typeof body.message === 'string'
        ? body.message
        : `Failed to ${action} agent: ${error.status}`
    },
  )
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

/**
 * Mutation: install BrowserOS as MCP into a single agent. Takes the
 * proxy-facing MCP URL so the agent's on-disk config records the
 * URL external clients can actually reach (NOT the agent server's
 * internal port — those differ in production where the browser
 * proxies `/mcp` to the agent server).
 *
 * Throws synchronously if `mcpUrl` is null. Omitting it would let
 * the server fall back to its own internal URL — exactly the wrong
 * value, since that's the bug this whole surface fixes. The Connect
 * button is disabled when the URL hasn't resolved yet, but this
 * guard means a stale render or programmatic caller can't bypass
 * that.
 */
export function useInstallAgent(mcpUrl: string | null) {
  const { baseUrl } = useAgentServerUrl()
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (agentId: string) => {
      if (!baseUrl) throw new Error('Agent server URL is unavailable')
      if (!mcpUrl) {
        throw new Error(
          'MCP server URL is not ready yet. Wait for the page to finish loading and try again.',
        )
      }
      return callMutation(baseUrl, agentId, 'install', { mcpUrl })
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
