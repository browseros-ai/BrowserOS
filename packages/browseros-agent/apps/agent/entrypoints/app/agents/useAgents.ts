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
  type HarnessQueuedMessage,
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

/**
 * Apply a partial update to a harness agent. Used by the pin-toggle
 * star and (eventually) the inline rename UI. Optimistically writes
 * the patch into the listing query cache so the row updates instantly,
 * then rolls back if the server rejects the change.
 */
export function useUpdateHarnessAgent() {
  const { baseUrl, isLoading: urlLoading } = useAgentServerUrl()
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: async (input: {
      agentId: string
      patch: { name?: string; pinned?: boolean }
    }) => {
      if (!baseUrl || urlLoading) {
        throw new Error('BrowserOS agent server URL is not ready')
      }
      const data = await agentsFetch<{ agent: HarnessAgent }>(
        baseUrl,
        `/${encodeURIComponent(input.agentId)}`,
        {
          method: 'PATCH',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(input.patch),
        },
      )
      return data.agent
    },
    onMutate: async ({ agentId, patch }) => {
      const queryKey = [AGENT_QUERY_KEYS.agents, baseUrl]
      await queryClient.cancelQueries({ queryKey })
      const previous = queryClient.getQueryData<HarnessAgentsResponse>(queryKey)
      if (!previous) return { previous: undefined }
      queryClient.setQueryData<HarnessAgentsResponse>(queryKey, {
        ...previous,
        agents: previous.agents.map((agent) =>
          agent.id === agentId ? { ...agent, ...patch } : agent,
        ),
      })
      return { previous }
    },
    onError: (_err, _vars, context) => {
      if (!context?.previous) return
      queryClient.setQueryData(
        [AGENT_QUERY_KEYS.agents, baseUrl],
        context.previous,
      )
    },
    onSettled: async () => {
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

/**
 * Subscribe to an existing turn (the server's `ActiveTurnRegistry`
 * decoupled the turn lifecycle from POST /chat). `lastSeq` lets the
 * client resume after a disconnect — the server replays buffered
 * frames with seq > lastSeq, then tails new ones.
 */
export async function attachToHarnessTurn(
  agentId: string,
  options: { turnId?: string; lastSeq?: number; signal?: AbortSignal } = {},
): Promise<Response> {
  const baseUrl = await getAgentServerUrl()
  const url = new URL(
    `${baseUrl}/agents/${encodeURIComponent(agentId)}/chat/stream`,
  )
  if (options.turnId) url.searchParams.set('turnId', options.turnId)
  const headers: Record<string, string> = {}
  if (typeof options.lastSeq === 'number') {
    headers['Last-Event-ID'] = String(options.lastSeq)
  }
  return fetch(url.toString(), { signal: options.signal, headers })
}

export interface HarnessActiveTurnInfo {
  turnId: string
  agentId: string
  sessionId: 'main'
  status: 'running' | 'done' | 'error' | 'cancelled'
  lastSeq: number
  startedAt: number
  endedAt?: number
  /** User message that kicked off the turn; null when not captured. */
  prompt: string | null
}

/**
 * Discover an in-flight turn for an agent. Used on chat mount so the
 * UI reattaches instead of starting a new turn after a tab/refresh.
 */
export async function fetchActiveHarnessTurn(
  agentId: string,
): Promise<HarnessActiveTurnInfo | null> {
  const baseUrl = await getAgentServerUrl()
  const response = await fetch(
    `${baseUrl}/agents/${encodeURIComponent(agentId)}/chat/active`,
  )
  if (!response.ok) return null
  const body = (await response.json()) as {
    active: HarnessActiveTurnInfo | null
  }
  return body.active
}

/**
 * Stop button. Hits the explicit cancel endpoint instead of just
 * aborting the fetch (which now only detaches *this* subscriber from
 * the buffer; the underlying turn would otherwise keep running).
 */
export async function cancelHarnessTurn(
  agentId: string,
  options: { turnId?: string; reason?: string } = {},
): Promise<{ cancelled: boolean }> {
  const baseUrl = await getAgentServerUrl()
  const response = await fetch(
    `${baseUrl}/agents/${encodeURIComponent(agentId)}/chat/cancel`,
    {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        ...(options.turnId ? { turnId: options.turnId } : {}),
        ...(options.reason ? { reason: options.reason } : {}),
      }),
    },
  )
  if (!response.ok) return { cancelled: false }
  return (await response.json()) as { cancelled: boolean }
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

export interface EnqueueMessageInput {
  message: string
  attachments?: ReadonlyArray<unknown>
}

export async function enqueueHarnessMessage(
  agentId: string,
  input: EnqueueMessageInput,
): Promise<HarnessQueuedMessage> {
  const baseUrl = await getAgentServerUrl()
  const response = await fetch(
    `${baseUrl}/agents/${encodeURIComponent(agentId)}/queue`,
    {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        message: input.message,
        ...(input.attachments && input.attachments.length > 0
          ? { attachments: input.attachments }
          : {}),
      }),
    },
  )
  if (!response.ok) {
    let message = `Request failed with status ${response.status}`
    try {
      const body = (await response.json()) as { error?: string }
      if (body.error) message = body.error
    } catch {}
    throw new Error(message)
  }
  const body = (await response.json()) as { queued: HarnessQueuedMessage }
  return body.queued
}

export async function removeHarnessQueuedMessage(
  agentId: string,
  messageId: string,
): Promise<{ removed: boolean }> {
  const baseUrl = await getAgentServerUrl()
  const response = await fetch(
    `${baseUrl}/agents/${encodeURIComponent(agentId)}/queue/${encodeURIComponent(
      messageId,
    )}`,
    { method: 'DELETE' },
  )
  if (!response.ok) return { removed: false }
  return (await response.json()) as { removed: boolean }
}

/**
 * Optimistic enqueue: writes the new queued message into the listing
 * cache immediately so the queue panel reflects the change without
 * waiting for the next poll. Rolls back if the server rejects.
 */
export function useEnqueueHarnessMessage() {
  const { baseUrl } = useAgentServerUrl()
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: async (input: { agentId: string } & EnqueueMessageInput) =>
      enqueueHarnessMessage(input.agentId, input),
    onMutate: async (input) => {
      const queryKey = [AGENT_QUERY_KEYS.agents, baseUrl]
      await queryClient.cancelQueries({ queryKey })
      const previous = queryClient.getQueryData<HarnessAgentsResponse>(queryKey)
      if (!previous) return { previous: undefined }
      const optimistic: HarnessQueuedMessage = {
        id: `optimistic-${Math.random().toString(36).slice(2, 10)}`,
        createdAt: Date.now(),
        message: input.message,
      }
      queryClient.setQueryData<HarnessAgentsResponse>(queryKey, {
        ...previous,
        agents: previous.agents.map((agent) =>
          agent.id === input.agentId
            ? { ...agent, queue: [...(agent.queue ?? []), optimistic] }
            : agent,
        ),
      })
      return { previous }
    },
    onError: (_err, _vars, context) => {
      if (!context?.previous) return
      queryClient.setQueryData(
        [AGENT_QUERY_KEYS.agents, baseUrl],
        context.previous,
      )
    },
    onSettled: async () => {
      await queryClient.invalidateQueries({
        queryKey: [AGENT_QUERY_KEYS.agents],
      })
    },
  })
}

/**
 * Optimistic queue removal mirror of `useEnqueueHarnessMessage`.
 */
export function useRemoveHarnessQueuedMessage() {
  const { baseUrl } = useAgentServerUrl()
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: async (input: { agentId: string; messageId: string }) =>
      removeHarnessQueuedMessage(input.agentId, input.messageId),
    onMutate: async (input) => {
      const queryKey = [AGENT_QUERY_KEYS.agents, baseUrl]
      await queryClient.cancelQueries({ queryKey })
      const previous = queryClient.getQueryData<HarnessAgentsResponse>(queryKey)
      if (!previous) return { previous: undefined }
      queryClient.setQueryData<HarnessAgentsResponse>(queryKey, {
        ...previous,
        agents: previous.agents.map((agent) =>
          agent.id === input.agentId
            ? {
                ...agent,
                queue: (agent.queue ?? []).filter(
                  (entry) => entry.id !== input.messageId,
                ),
              }
            : agent,
        ),
      })
      return { previous }
    },
    onError: (_err, _vars, context) => {
      if (!context?.previous) return
      queryClient.setQueryData(
        [AGENT_QUERY_KEYS.agents, baseUrl],
        context.previous,
      )
    },
    onSettled: async () => {
      await queryClient.invalidateQueries({
        queryKey: [AGENT_QUERY_KEYS.agents],
      })
    },
  })
}
