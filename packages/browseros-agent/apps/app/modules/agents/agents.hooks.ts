import type { AgentRoutes } from '@browseros/server'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { hc } from 'hono/client'
import { Feature } from '@/lib/browseros/capabilities'
import { useAgentServerUrl } from '@/modules/browseros/agent-server-url.hooks'
import { useCapabilities } from '@/modules/browseros/capabilities.hooks'
import type { AcpAgentType } from './acp-agent-types'
import { computeAgentsSettled } from './agents.helpers'

interface CreateAcpAgentInput {
  name: string
  type: AcpAgentType
  modelId?: string
  reasoningEffort?: string
}

const AGENTS_QUERY_KEY = 'acp-agents'

function agentsClient(baseUrl: string) {
  return hc<AgentRoutes>(`${baseUrl}/agents`)
}

async function agentRequestError(response: Response): Promise<Error> {
  const body = (await response.json().catch(() => ({}))) as { error?: string }
  return new Error(
    body.error ?? `Request failed with status ${response.status}`,
  )
}

export function useAcpAgents(enabled = true) {
  const { supports, isLoading: capabilitiesLoading } = useCapabilities()
  const agentsSupported = supports(Feature.AGENT_HARNESS_SUPPORT)
  const {
    baseUrl,
    isLoading: urlLoading,
    error: urlError,
  } = useAgentServerUrl()
  const query = useQuery({
    queryKey: [AGENTS_QUERY_KEY, baseUrl],
    queryFn: async () => {
      const response = await agentsClient(baseUrl as string).index.$get()
      if (!response.ok) throw await agentRequestError(response)
      return response.json()
    },
    enabled: Boolean(baseUrl) && !urlLoading && enabled && agentsSupported,
  })

  return {
    agents: agentsSupported ? (query.data?.agents ?? []) : [],
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
    mutationFn: async (input: CreateAcpAgentInput) => {
      if (!baseUrl || isLoading) {
        throw new Error('BrowserOS agent server URL is not ready')
      }
      const response = await agentsClient(baseUrl).index.$post({ json: input })
      if (!response.ok) throw await agentRequestError(response)
      const result = await response.json()
      return result.agent
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
      const response = await agentsClient(baseUrl)[':agentId'].$delete({
        param: { agentId },
      })
      if (!response.ok) throw await agentRequestError(response)
      return response.json()
    },
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: [AGENTS_QUERY_KEY] }),
  })
}
