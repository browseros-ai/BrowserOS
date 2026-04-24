import { useQuery } from '@tanstack/react-query'
import { useAgentServerUrl } from '@/lib/browseros/useBrowserOSProviders'

export interface AgentOverview {
  agentId: string
  latestMessage: string | null
  latestMessageAt: number | null
  activitySummary: string | null
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

export function useAgentDashboard(enabled: boolean) {
  const { baseUrl, isLoading: urlLoading } = useAgentServerUrl()

  return useQuery<DashboardResponse>({
    queryKey: ['claw', 'dashboard', baseUrl],
    queryFn: async () => {
      const url = new URL('/claw/dashboard', baseUrl as string)
      const response = await fetch(url.toString())
      if (!response.ok) throw new Error('Failed to fetch dashboard')
      return response.json()
    },
    enabled: enabled && Boolean(baseUrl) && !urlLoading,
    refetchInterval: 10_000,
    staleTime: 5_000,
  })
}
