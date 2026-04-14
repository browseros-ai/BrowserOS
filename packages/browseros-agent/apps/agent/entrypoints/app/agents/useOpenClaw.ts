import { useEffect, useState } from 'react'
import { getAgentServerUrl } from '@/lib/browseros/helpers'

export interface AgentEntry {
  agentId: string
  name: string
  workspace: string
  model?: unknown
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

async function clawFetch<T>(path: string, init?: RequestInit): Promise<T> {
  const baseUrl = await getAgentServerUrl()
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

export function useOpenClawStatus(pollMs = 5000) {
  const [status, setStatus] = useState<OpenClawStatus | null>(null)
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    let active = true
    const poll = async () => {
      try {
        const s = await clawFetch<OpenClawStatus>('/status')
        if (active) setStatus(s)
      } catch {
        // Server may not be running
      } finally {
        if (active) setLoading(false)
      }
    }
    poll()
    const id = setInterval(poll, pollMs)
    return () => {
      active = false
      clearInterval(id)
    }
  }, [pollMs])

  return { status, loading }
}

export function useOpenClawAgents(refreshKey: number) {
  const [agents, setAgents] = useState<AgentEntry[]>([])
  const [loading, setLoading] = useState(true)

  // biome-ignore lint/correctness/useExhaustiveDependencies: refreshKey is an intentional refetch trigger
  useEffect(() => {
    let active = true
    clawFetch<{ agents: AgentEntry[] }>('/agents')
      .then((data) => {
        if (active) setAgents(data.agents ?? [])
      })
      .catch(() => {})
      .finally(() => {
        if (active) setLoading(false)
      })
    return () => {
      active = false
    }
  }, [refreshKey])

  return { agents, loading }
}

export async function setupOpenClaw(input: {
  providerType?: string
  providerName?: string
  baseUrl?: string
  apiKey?: string
  modelId?: string
}) {
  return clawFetch<{ status: string; agents: AgentEntry[] }>('/setup', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(input),
  })
}

export async function createAgent(input: {
  name: string
  providerType?: string
  providerName?: string
  baseUrl?: string
  apiKey?: string
  modelId?: string
}) {
  return clawFetch<{ agent: AgentEntry }>('/agents', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(input),
  })
}

export async function deleteAgent(id: string) {
  return clawFetch<{ success: boolean }>(`/agents/${id}`, {
    method: 'DELETE',
  })
}

export async function startOpenClaw() {
  return clawFetch<{ status: string }>('/start', { method: 'POST' })
}

export async function stopOpenClaw() {
  return clawFetch<{ status: string }>('/stop', { method: 'POST' })
}

export async function restartOpenClaw() {
  return clawFetch<{ status: string }>('/restart', { method: 'POST' })
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
