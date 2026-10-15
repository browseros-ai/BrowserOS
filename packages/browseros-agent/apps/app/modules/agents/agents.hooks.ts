import type {
  CreateAgentRequest,
  UpdateAgentRequest,
} from '@browseros/server-api'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Feature } from '@/lib/browseros/capabilities'
import { serverApi, serverData } from '@/lib/server-api/client'
import { useAgentServerUrl } from '@/modules/browseros/agent-server-url.hooks'
import { useCapabilities } from '@/modules/browseros/capabilities.hooks'
import { computeAgentsSettled } from './agents.helpers'

type CreateAcpAgentInput = CreateAgentRequest
interface UpdateAcpAgentInput {
  agentId: string
  patch: UpdateAgentRequest
}

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
    queryKey: [AGENTS_QUERY_KEY, baseUrl],
    queryFn: async () => {
      return serverData(serverApi(baseUrl as string).http.GET('/agents'))
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
      const result = await serverData(
        serverApi(baseUrl).http.POST('/agents', { body: input }),
      )
      return result.agent
    },
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: [AGENTS_QUERY_KEY] }),
  })
}

export function useUpdateAcpAgent() {
  const { baseUrl, isLoading } = useAgentServerUrl()
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: async ({ agentId, patch }: UpdateAcpAgentInput) => {
      if (!baseUrl || isLoading) {
        throw new Error('BrowserOS agent server URL is not ready')
      }
      const result = await serverData(
        serverApi(baseUrl).http.PUT('/agents/{agentId}', {
          params: { path: { agentId } },
          body: patch,
        }),
      )
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
      return serverData(
        serverApi(baseUrl).http.DELETE('/agents/{agentId}', {
          params: { path: { agentId } },
        }),
      )
    },
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: [AGENTS_QUERY_KEY] }),
  })
}
