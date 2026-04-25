import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useEffect } from 'react'
import { useAgentServerUrl } from '@/lib/browseros/useBrowserOSProviders'

export interface AgentOverview {
  agentId: string
  status: 'working' | 'idle' | 'error' | 'unknown'
  latestMessage: string | null
  latestMessageAt: number | null
  activitySummary: string | null
  currentTool: string | null
  totalCostUsd: number
  sessionCount: number
}

export interface DashboardResponse {
  agents: AgentOverview[]
  summary: {
    totalAgents: number
    totalCostUsd: number
  }
}

interface StatusEvent {
  agentId: string
  status: AgentOverview['status']
  currentTool: string | null
  error: string | null
  timestamp: number
}

const DASHBOARD_QUERY_KEY = ['claw', 'dashboard']

export function useAgentDashboard(enabled: boolean) {
  const { baseUrl, isLoading: urlLoading } = useAgentServerUrl()
  const queryClient = useQueryClient()
  const ready = enabled && Boolean(baseUrl) && !urlLoading

  // Initial data load + periodic refresh as fallback
  const query = useQuery<DashboardResponse>({
    queryKey: [...DASHBOARD_QUERY_KEY, baseUrl],
    queryFn: async () => {
      const url = new URL('/claw/dashboard', baseUrl as string)
      const response = await fetch(url.toString())
      if (!response.ok) throw new Error('Failed to fetch dashboard')
      return response.json()
    },
    enabled: ready,
  })

  // SSE subscription for real-time status patches
  useEffect(() => {
    if (!ready || !baseUrl) return

    const streamUrl = new URL('/claw/dashboard/stream', baseUrl)
    const eventSource = new EventSource(streamUrl.toString())

    eventSource.addEventListener('snapshot', (event) => {
      try {
        const dashboard = JSON.parse(event.data) as DashboardResponse
        queryClient.setQueryData([...DASHBOARD_QUERY_KEY, baseUrl], dashboard)
      } catch {}
    })

    eventSource.addEventListener('status', (event) => {
      try {
        const status = JSON.parse(event.data) as StatusEvent
        queryClient.setQueryData<DashboardResponse>(
          [...DASHBOARD_QUERY_KEY, baseUrl],
          (prev) => {
            if (!prev) return prev
            return {
              ...prev,
              agents: prev.agents.map((agent) =>
                agent.agentId === status.agentId
                  ? {
                      ...agent,
                      status: status.status,
                      currentTool: status.currentTool,
                    }
                  : agent,
              ),
            }
          },
        )
      } catch {}
    })

    return () => {
      eventSource.close()
    }
  }, [ready, baseUrl, queryClient])

  return query
}
