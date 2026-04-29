import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { getAgentServerUrl } from '@/lib/browseros/helpers'
import { useAgentServerUrl } from '@/lib/browseros/useBrowserOSProviders'
import { buildAgentApiUrl } from './agent-api-url'
import {
  type AgentHarnessStreamEvent,
  type CreateHarnessAgentInput,
  type HarnessAdapterDescriptor,
  type HarnessAgent,
  type HarnessAgentHistoryPage,
  mapHarnessAgentToEntry,
} from './agent-harness-types'
import type { OpenClawStatus } from './useOpenClaw'

/**
 * Combined response shape of `GET /agents`. The page polls this once
 * and consumes both fields, replacing the dedicated `/claw/status`
 * poll the previous design carried.
 */
interface HarnessAgentsResponse {
  agents: HarnessAgent[]
  gateway: OpenClawStatus | null
}

export type { AgentHarnessStreamEvent }

const AGENT_QUERY_KEYS = {
  adapters: 'agent-harness-adapters',
  agents: 'agent-harness-agents',
} as const

async function agentsFetch<T>(
  baseUrl: string,
  path: string,
  init?: RequestInit,
): Promise<T> {
  const res = await fetch(buildAgentApiUrl(baseUrl, path), init)
  if (!res.ok) {
    let message = `Request failed with status ${res.status}`
    try {
      const body = (await res.json()) as { error?: string }
      if (body.error) message = body.error
    } catch {}
    throw new Error(message)
  }
  return res.json() as Promise<T>
}

export function useAgentAdapters(enabled = true) {
  const {
    baseUrl,
    isLoading: urlLoading,
    error: urlError,
  } = useAgentServerUrl()

  const query = useQuery<HarnessAdapterDescriptor[], Error>({
    queryKey: [AGENT_QUERY_KEYS.adapters, baseUrl],
    queryFn: async () => {
      const data = await agentsFetch<{ adapters: HarnessAdapterDescriptor[] }>(
        baseUrl as string,
        '/adapters',
      )
      return data.adapters ?? []
    },
    enabled: Boolean(baseUrl) && !urlLoading && enabled,
  })

  return {
    adapters: query.data ?? [],
    loading: query.isLoading || urlLoading,
    error: query.error ?? urlError,
    refetch: query.refetch,
  }
}

export function useHarnessAgents(enabled = true) {
  const {
    baseUrl,
    isLoading: urlLoading,
    error: urlError,
  } = useAgentServerUrl()

  const query = useQuery<HarnessAgentsResponse, Error>({
    queryKey: [AGENT_QUERY_KEYS.agents, baseUrl],
    queryFn: async () => {
      const data = await agentsFetch<HarnessAgentsResponse>(
        baseUrl as string,
        '/',
      )
      return {
        agents: data.agents ?? [],
        gateway: data.gateway ?? null,
      }
    },
    enabled: Boolean(baseUrl) && !urlLoading && enabled,
    // Poll every 5s so the per-agent liveness state (working / idle /
    // asleep / error) and last-used timestamps stay fresh without a
    // websocket. `refetchIntervalInBackground: false` lets a hidden
    // tab go quiet — react-query's default, made explicit.
    refetchInterval: 5_000,
    refetchIntervalInBackground: false,
  })

  return {
    agents: (query.data?.agents ?? []).map(mapHarnessAgentToEntry),
    harnessAgents: query.data?.agents ?? [],
    gateway: query.data?.gateway ?? null,
    loading: query.isLoading || urlLoading,
    error: query.error ?? urlError,
    refetch: query.refetch,
  }
}

export function useCreateHarnessAgent() {
  const { baseUrl, isLoading: urlLoading } = useAgentServerUrl()
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: async (input: CreateHarnessAgentInput) => {
      if (!baseUrl || urlLoading) {
        throw new Error('BrowserOS agent server URL is not ready')
      }
      const data = await agentsFetch<{ agent: HarnessAgent }>(baseUrl, '/', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(input),
      })
      return data.agent
    },
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: [AGENT_QUERY_KEYS.agents],
      })
    },
  })
}

export function useDeleteHarnessAgent() {
  const { baseUrl, isLoading: urlLoading } = useAgentServerUrl()
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: async (agentId: string) => {
      if (!baseUrl || urlLoading) {
        throw new Error('BrowserOS agent server URL is not ready')
      }
      return agentsFetch<{ success: boolean }>(
        baseUrl,
        `/${encodeURIComponent(agentId)}`,
        { method: 'DELETE' },
      )
    },
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: [AGENT_QUERY_KEYS.agents],
      })
    },
  })
}

export async function chatWithHarnessAgent(
  agentId: string,
  message: string,
  signal?: AbortSignal,
  attachments?: ReadonlyArray<unknown>,
): Promise<Response> {
  const baseUrl = await getAgentServerUrl()
  return fetch(`${baseUrl}/agents/${encodeURIComponent(agentId)}/chat`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      message,
      ...(attachments && attachments.length > 0 ? { attachments } : {}),
    }),
    signal,
  })
}

export async function fetchHarnessAgentHistory(
  agentId: string,
): Promise<HarnessAgentHistoryPage> {
  const baseUrl = await getAgentServerUrl()
  return agentsFetch<HarnessAgentHistoryPage>(
    baseUrl,
    `/${encodeURIComponent(agentId)}/sessions/main/history`,
  )
}
