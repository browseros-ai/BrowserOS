import { ServerApiClient } from '@browseros/server-api/client'
import type { components } from '@browseros/server-api/openapi'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Feature } from '@/lib/browseros/capabilities'
import { agentServerReadRetry } from '@/modules/browseros/agent-server-read-retry'
import { useAgentServerUrl } from '@/modules/browseros/agent-server-url.hooks'
import { useCapabilities } from '@/modules/browseros/capabilities.hooks'
import { computeAgentsSettled } from './agents.helpers'

const AGENTS_QUERY_KEY = 'acp-agents'

export function useAcpAgents(enabled = true) {
  const { supports, isLoading: capabilitiesLoading } = useCapabilities()
  const agentsSupported = supports(Feature.AGENT_HARNESS_SUPPORT)
  const {
    baseUrl,
    isLoading: urlLoading,
    error: urlError,
  } = useAgentServerUrl()
  const query = useQuery({
    ...agentServerReadRetry,
    queryKey: [AGENTS_QUERY_KEY, baseUrl],
    queryFn: async () => {
      if (!baseUrl) throw new Error('BrowserOS agent server URL is not ready')
      const { agents } = await new ServerApiClient(baseUrl).listAgents()
      // The TypeScript server can still return custom ACP rows. Only the two
      // built-in agents are supported by the OpenAPI contract.
      return agents.filter(({ type }) => type === 'claude' || type === 'codex')
    },
    enabled: Boolean(baseUrl) && !urlLoading && enabled && agentsSupported,
  })

  return {
    agents: agentsSupported ? (query.data ?? []) : [],
    loading:
      capabilitiesLoading ||
      (agentsSupported && (query.isLoading || urlLoading)),
    // `loading` (via query.isLoading) briefly reads false on the render the
    // query flips enabled, while `agents` is still empty. `settled` instead
    // stays false until the fetch has succeeded, so callers can tell a
    // not-yet-loaded (or failed) agent list from a genuinely absent one.
    settled: computeAgentsSettled({
      capabilitiesLoading,
      agentsSupported,
      urlLoading,
      agentsQuerySucceeded: query.isSuccess,
    }),
    error: agentsSupported ? (query.error ?? urlError) : null,
    refetch: query.refetch,
  }
}

export function useCreateAcpAgent() {
  const { baseUrl, isLoading } = useAgentServerUrl()
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: async (input: components['schemas']['AgentInput']) => {
      if (!baseUrl || isLoading) {
        throw new Error('BrowserOS agent server URL is not ready')
      }
      const { agent } = await new ServerApiClient(baseUrl).createAgent(input)
      return agent
    },
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: [AGENTS_QUERY_KEY] }),
  })
}

export function useDeleteAcpAgent() {
  const { baseUrl, isLoading } = useAgentServerUrl()
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: async (agentId: string) => {
      if (!baseUrl || isLoading) {
        throw new Error('BrowserOS agent server URL is not ready')
      }
      return new ServerApiClient(baseUrl).deleteAgent({ agentId })
    },
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: [AGENTS_QUERY_KEY] }),
  })
}
