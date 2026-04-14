import type {
  BrowserOSAgentRoleId,
  BrowserOSCustomRoleInput,
} from '@browseros/shared/types/role-aware-agents'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { getAgentServerUrl } from '@/lib/browseros/helpers'
import { useAgentServerUrl } from '@/lib/browseros/useBrowserOSProviders'

export interface AgentEntry {
  agentId: string
  name: string
  workspace: string
  model?: unknown
  role?: {
    roleSource: 'builtin' | 'custom'
    roleId?: BrowserOSAgentRoleId
    roleName: string
    shortDescription: string
  }
}

export interface RoleTemplateSummary {
  id: BrowserOSAgentRoleId
  name: string
  shortDescription: string
  longDescription: string
  recommendedApps: string[]
  defaultAgentName: string
  boundaries: Array<{
    key: string
    label: string
    description: string
    defaultMode: 'allow' | 'ask' | 'block'
  }>
}

export function getModelDisplayName(model: unknown): string | undefined {
  if (typeof model === 'string') return model.split('/').pop()
  return undefined
}

export interface OpenClawStatus {
  status: 'uninitialized' | 'starting' | 'running' | 'stopped' | 'error'
  podmanAvailable: boolean
  machineReady: boolean
  port: number | null
  agentCount: number
  error: string | null
}

export const OPENCLAW_QUERY_KEYS = {
  status: 'openclaw-status',
  agents: 'openclaw-agents',
  roles: 'openclaw-roles',
} as const

async function clawFetch<T>(baseUrl: string, path: string, init?: RequestInit) {
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

async function fetchOpenClawAgents(
  baseUrl: string,
): Promise<{ agents: AgentEntry[] }> {
  return clawFetch<{ agents: AgentEntry[] }>(baseUrl, '/agents')
}

async function fetchOpenClawRoles(
  baseUrl: string,
): Promise<{ roles: RoleTemplateSummary[] }> {
  return clawFetch<{ roles: RoleTemplateSummary[] }>(baseUrl, '/roles')
}

export function useOpenClawStatus(pollMs = 5000) {
  const { baseUrl, isLoading: urlLoading } = useAgentServerUrl()
  const query = useQuery<OpenClawStatus, Error>({
    queryKey: [OPENCLAW_QUERY_KEYS.status, baseUrl],
    queryFn: () => fetchOpenClawStatus(baseUrl as string),
    enabled: !!baseUrl && !urlLoading,
    refetchInterval: pollMs,
    refetchOnWindowFocus: true,
  })

  return {
    status: query.data ?? null,
    loading: query.isLoading || urlLoading,
    error: query.error,
    refetch: query.refetch,
  }
}

export function useOpenClawAgents() {
  const { baseUrl, isLoading: urlLoading } = useAgentServerUrl()
  const query = useQuery<{ agents: AgentEntry[] }, Error>({
    queryKey: [OPENCLAW_QUERY_KEYS.agents, baseUrl],
    queryFn: () => fetchOpenClawAgents(baseUrl as string),
    enabled: !!baseUrl && !urlLoading,
    refetchOnWindowFocus: true,
  })

  return {
    agents: query.data?.agents ?? [],
    loading: query.isLoading || urlLoading,
    error: query.error,
    refetch: query.refetch,
  }
}

export function useOpenClawRoles() {
  const { baseUrl, isLoading: urlLoading } = useAgentServerUrl()
  const query = useQuery<{ roles: RoleTemplateSummary[] }, Error>({
    queryKey: [OPENCLAW_QUERY_KEYS.roles, baseUrl],
    queryFn: () => fetchOpenClawRoles(baseUrl as string),
    enabled: !!baseUrl && !urlLoading,
    staleTime: 60_000,
  })

  return {
    roles: query.data?.roles ?? [],
    loading: query.isLoading || urlLoading,
    error: query.error,
    refetch: query.refetch,
  }
}

export function useInvalidateOpenClawQueries() {
  const queryClient = useQueryClient()

  return async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: [OPENCLAW_QUERY_KEYS.status] }),
      queryClient.invalidateQueries({ queryKey: [OPENCLAW_QUERY_KEYS.agents] }),
    ])
  }
}

export interface OpenClawAgentMutationInput {
  name: string
  roleId?: BrowserOSAgentRoleId
  customRole?: BrowserOSCustomRoleInput
  providerType?: string
  providerName?: string
  baseUrl?: string
  apiKey?: string
  modelId?: string
}

export interface OpenClawSetupInput {
  providerType?: string
  providerName?: string
  baseUrl?: string
  apiKey?: string
  modelId?: string
}

export function useSetupOpenClawMutation() {
  const { baseUrl } = useAgentServerUrl()
  return useMutation({
    mutationFn: (input: OpenClawSetupInput) =>
      clawFetch<{ status: string; agents: AgentEntry[] }>(
        baseUrl as string,
        '/setup',
        {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(input),
        },
      ),
  })
}

export function useCreateOpenClawAgentMutation() {
  const { baseUrl } = useAgentServerUrl()
  return useMutation({
    mutationFn: (input: OpenClawAgentMutationInput) =>
      clawFetch<{ agent: AgentEntry }>(baseUrl as string, '/agents', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(input),
      }),
  })
}

export function useDeleteOpenClawAgentMutation() {
  const { baseUrl } = useAgentServerUrl()
  return useMutation({
    mutationFn: (id: string) =>
      clawFetch<{ success: boolean }>(baseUrl as string, `/agents/${id}`, {
        method: 'DELETE',
      }),
  })
}

export function useStartOpenClawMutation() {
  const { baseUrl } = useAgentServerUrl()
  return useMutation({
    mutationFn: () =>
      clawFetch<{ status: string }>(baseUrl as string, '/start', {
        method: 'POST',
      }),
  })
}

export function useStopOpenClawMutation() {
  const { baseUrl } = useAgentServerUrl()
  return useMutation({
    mutationFn: () =>
      clawFetch<{ status: string }>(baseUrl as string, '/stop', {
        method: 'POST',
      }),
  })
}

export function useRestartOpenClawMutation() {
  const { baseUrl } = useAgentServerUrl()
  return useMutation({
    mutationFn: () =>
      clawFetch<{ status: string }>(baseUrl as string, '/restart', {
        method: 'POST',
      }),
  })
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

export async function chatWithAgent(
  agentId: string,
  message: string,
  sessionKey?: string,
  signal?: AbortSignal,
): Promise<Response> {
  const baseUrl = await getAgentServerUrl()
  return fetch(`${baseUrl}/claw/agents/${agentId}/chat`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ message, sessionKey }),
    signal,
  })
}
