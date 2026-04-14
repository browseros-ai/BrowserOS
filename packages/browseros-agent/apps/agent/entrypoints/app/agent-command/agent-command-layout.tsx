import type { FC } from 'react'
import { Outlet, useOutletContext } from 'react-router'
import {
  type AgentEntry,
  type OpenClawStatus,
  useOpenClawAgents,
  useOpenClawStatus,
} from '@/entrypoints/app/agents/useOpenClaw'

interface AgentCommandContextValue {
  agents: AgentEntry[]
  agentsLoading: boolean
  status: OpenClawStatus | null
  statusLoading: boolean
}

export const AgentCommandLayout: FC = () => {
  const { status, loading: statusLoading } = useOpenClawStatus(5000)
  const { agents, loading: agentsLoading } = useOpenClawAgents(
    status?.status === 'running' && status.controlPlaneStatus === 'connected',
  )

  return (
    <Outlet
      context={
        {
          agents,
          agentsLoading,
          status,
          statusLoading,
        } satisfies AgentCommandContextValue
      }
    />
  )
}

export function useAgentCommandData(): AgentCommandContextValue {
  return useOutletContext<AgentCommandContextValue>()
}
