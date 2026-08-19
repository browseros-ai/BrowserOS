import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Feature } from '@/lib/browseros/capabilities'
import { useAgentServerUrl } from '@/modules/browseros/agent-server-url.hooks'
import { useCapabilities } from '@/modules/browseros/capabilities.hooks'
import type { AcpAgent, AcpAgentType } from './acp-agent-types'
import { buildAgentApiUrl } from './agent-api-url'
import { computeAgentsSettled } from './agents.helpers'

interface AcpAgentsResponse {
  agents: AcpAgent[]
}

interface CreateAcpAgentInput {
  name: string
  type: AcpAgentType
  modelId?: string
  reasoningEffort?: string
}

const AGENTS_QUERY_KEY = 'acp-agents'

async function agentsFetch<T>(
  baseUrl: string,
  path: string,
  init?: RequestInit,
): Promise<T> {
  const response = await fetch(buildAgentApiUrl(baseUrl, path), init)
  if (!response.ok) {
    const body = (await response.json().catch(() => ({}))) as {
      error?: string
    }
    throw new Error(
      body.error ?? `Request failed with status ${response.status}`,
    )
  }
  return response.json() as Promise<T>
}

export function useAcpAgents(enabled = true) {
  const { supports, isLoading: capabilitiesLoading } = useCapabilities()
  const agentsSupported = supports(Feature.AGENT_HARNESS_SUPPORT)
  const {
    baseUrl,
    isLoading: urlLoading,
    error: urlError,
  } = useAgentServerUrl()
  const query = useQuery<AcpAgentsResponse, Error>({
    queryKey: [AGENTS_QUERY_KEY, baseUrl],
    queryFn: () => agentsFetch(baseUrl as string, '/'),
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
      const result = await agentsFetch<{ agent: AcpAgent }>(baseUrl, '/', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(input),
      })
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
      return agentsFetch<{ success: true }>(
        baseUrl,
        `/${encodeURIComponent(agentId)}`,
        { method: 'DELETE' },
      )
    },
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: [AGENTS_QUERY_KEY] }),
  })
}
