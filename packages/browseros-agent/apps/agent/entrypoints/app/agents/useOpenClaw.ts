import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useAgentServerUrl } from '@/lib/browseros/useBrowserOSProviders'

export interface AgentEntry {
  agentId: string
  name: string
  workspace: string
  model?: unknown
  source?: 'openclaw' | 'agent-harness'
}

export interface OpenClawStatus {
  status: 'uninitialized' | 'starting' | 'running' | 'stopped' | 'error'
  podmanAvailable: boolean
  machineReady: boolean
  port: number | null
  agentCount: number
  error: string | null
  controlPlaneStatus:
    | 'disconnected'
    | 'connecting'
    | 'connected'
    | 'reconnecting'
    | 'recovering'
    | 'failed'
  lastGatewayError: string | null
  lastRecoveryReason:
    | 'transient_disconnect'
    | 'signature_expired'
    | 'pairing_required'
    | 'token_mismatch'
    | 'container_not_ready'
    | 'unknown'
    | null
}

export interface OpenClawAgentMutationInput {
  name: string
  providerType?: string
  providerName?: string
  baseUrl?: string
  apiKey?: string
  modelId?: string
  supportsImages?: boolean
}

export interface OpenClawSetupInput {
  providerType?: string
  providerName?: string
  baseUrl?: string
  apiKey?: string
  modelId?: string
  // Mirrors LlmProviderConfig.supportsImages — pass-through so the gateway
  // can declare the model's input modalities correctly when persisting the
  // custom-provider config.
  supportsImages?: boolean
}

export function getModelDisplayName(model: unknown): string | undefined {
  if (typeof model === 'string') return model.split('/').pop()
  return undefined
}

export const OPENCLAW_QUERY_KEYS = {
  status: 'openclaw-status',
  agents: 'openclaw-agents',
} as const

export type GatewayLifecycleAction =
  | 'setup'
  | 'start'
  | 'stop'
  | 'restart'
  | 'reconnect'

async function clawFetch<T>(
  baseUrl: string,
  path: string,
  init?: RequestInit,
): Promise<T> {
  const res = await fetch(`${baseUrl}/claw${path}`, init)
  if (!res.ok) {
    let message = `Request failed with status ${res.status}`
    try {
      const body = (await res.json()) as { error?: string }
      if (body.error) {
        message = body.error
      }
    } catch {}
    throw new Error(message)
  }
  return res.json() as Promise<T>
}

async function fetchOpenClawStatus(baseUrl: string): Promise<OpenClawStatus> {
  return clawFetch<OpenClawStatus>(baseUrl, '/status')
}

async function fetchOpenClawAgents(baseUrl: string): Promise<AgentEntry[]> {
  const data = await clawFetch<{ agents: AgentEntry[] }>(baseUrl, '/agents')
  return (data.agents ?? []).map((agent) => ({
    ...agent,
    source: 'openclaw',
  }))
}

async function invalidateOpenClawQueries(
  queryClient: ReturnType<typeof useQueryClient>,
): Promise<void> {
  await Promise.all([
    queryClient.invalidateQueries({ queryKey: [OPENCLAW_QUERY_KEYS.status] }),
    queryClient.invalidateQueries({ queryKey: [OPENCLAW_QUERY_KEYS.agents] }),
  ])
}

export function useOpenClawStatus(pollMs = 5000) {
  const {
    baseUrl,
    isLoading: urlLoading,
    error: urlError,
  } = useAgentServerUrl()

  const query = useQuery<OpenClawStatus, Error>({
    queryKey: [OPENCLAW_QUERY_KEYS.status, baseUrl],
    queryFn: () => fetchOpenClawStatus(baseUrl as string),
    enabled: !!baseUrl && !urlLoading,
    refetchInterval: pollMs,
  })

  return {
    status: query.data ?? null,
    loading: query.isLoading || urlLoading,
    error: query.error ?? urlError,
    refetch: query.refetch,
  }
}

export function useOpenClawAgents(enabled = true) {
  const {
    baseUrl,
    isLoading: urlLoading,
    error: urlError,
  } = useAgentServerUrl()

  const query = useQuery<AgentEntry[], Error>({
    queryKey: [OPENCLAW_QUERY_KEYS.agents, baseUrl],
    queryFn: () => fetchOpenClawAgents(baseUrl as string),
    enabled: !!baseUrl && !urlLoading && enabled,
  })

  return {
    agents: query.data ?? [],
    loading: query.isLoading || urlLoading,
    error: query.error ?? urlError,
    refetch: query.refetch,
  }
}

export function useOpenClawMutations() {
  const { baseUrl, isLoading: urlLoading } = useAgentServerUrl()
  const queryClient = useQueryClient()

  const ensureBaseUrl = () => {
    if (!baseUrl || urlLoading) {
      throw new Error('BrowserOS agent server URL is not ready')
    }
    return baseUrl
  }

  const onSuccess = () => invalidateOpenClawQueries(queryClient)

  const setupMutation = useMutation({
    mutationFn: async (input: OpenClawSetupInput) =>
      clawFetch<{ status: string; agents: AgentEntry[] }>(
        ensureBaseUrl(),
        '/setup',
        {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(input),
        },
      ),
    onSuccess,
  })

  const createMutation = useMutation({
    mutationFn: async (input: OpenClawAgentMutationInput) =>
      clawFetch<{ agent: AgentEntry }>(ensureBaseUrl(), '/agents', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(input),
      }),
    onSuccess,
  })

  const deleteMutation = useMutation({
    mutationFn: async (id: string) =>
      clawFetch<{ success: boolean }>(ensureBaseUrl(), `/agents/${id}`, {
        method: 'DELETE',
      }),
    onSuccess,
  })

  const startMutation = useMutation({
    mutationFn: async () =>
      clawFetch<{ status: string }>(ensureBaseUrl(), '/start', {
        method: 'POST',
      }),
    onSuccess,
  })

  const stopMutation = useMutation({
    mutationFn: async () =>
      clawFetch<{ status: string }>(ensureBaseUrl(), '/stop', {
        method: 'POST',
      }),
    onSuccess,
  })

  const restartMutation = useMutation({
    mutationFn: async () =>
      clawFetch<{ status: string }>(ensureBaseUrl(), '/restart', {
        method: 'POST',
      }),
    onSuccess,
  })

  const reconnectMutation = useMutation({
    mutationFn: async () =>
      clawFetch<{ status: string }>(ensureBaseUrl(), '/reconnect', {
        method: 'POST',
      }),
    onSuccess,
  })

  let pendingGatewayAction: GatewayLifecycleAction | null = null
  if (setupMutation.isPending) pendingGatewayAction = 'setup'
  else if (restartMutation.isPending) pendingGatewayAction = 'restart'
  else if (stopMutation.isPending) pendingGatewayAction = 'stop'
  else if (startMutation.isPending) pendingGatewayAction = 'start'
  else if (reconnectMutation.isPending) pendingGatewayAction = 'reconnect'

  return {
    setupOpenClaw: setupMutation.mutateAsync,
    createAgent: createMutation.mutateAsync,
    deleteAgent: deleteMutation.mutateAsync,
    startOpenClaw: startMutation.mutateAsync,
    stopOpenClaw: stopMutation.mutateAsync,
    restartOpenClaw: restartMutation.mutateAsync,
    reconnectOpenClaw: reconnectMutation.mutateAsync,
    actionInProgress:
      setupMutation.isPending ||
      createMutation.isPending ||
      deleteMutation.isPending ||
      startMutation.isPending ||
      stopMutation.isPending ||
      restartMutation.isPending ||
      reconnectMutation.isPending,
    settingUp: setupMutation.isPending,
    creating: createMutation.isPending,
    deleting: deleteMutation.isPending,
    reconnecting: reconnectMutation.isPending,
    pendingGatewayAction,
  }
}

export interface OpenClawStreamEvent {
  type:
    | 'text-delta'
    | 'thinking'
    | 'tool-start'
    | 'tool-end'
    | 'tool-output'
    | 'lifecycle'
    | 'done'
    | 'error'
  data: Record<string, unknown>
}

export interface OpenClawChatHistoryMessage {
  role: 'user' | 'assistant'
  content: string
}

interface ChatHistoryTurnLike {
  userText: string
  parts: Array<{ kind: string; text?: string }>
}

export function buildChatHistoryFromTurns(
  turns: ChatHistoryTurnLike[],
): OpenClawChatHistoryMessage[] {
  const messages: OpenClawChatHistoryMessage[] = []

  for (const turn of turns) {
    const userText = turn.userText.trim()
    if (userText) {
      messages.push({ role: 'user', content: userText })
    }

    const assistantText = turn.parts
      .filter(
        (
          part,
        ): part is {
          kind: 'text'
          text: string
        } => part.kind === 'text' && typeof part.text === 'string',
      )
      .map((part) => part.text.trim())
      .filter(Boolean)
      .join('\n\n')

    if (assistantText) {
      messages.push({ role: 'assistant', content: assistantText })
    }
  }

  return messages
}
