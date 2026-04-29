import type { FC } from 'react'
import { Outlet, useOutletContext } from 'react-router'
import { useHarnessAgents } from '@/entrypoints/app/agents/useAgents'
import type {
  AgentEntry,
  OpenClawStatus,
} from '@/entrypoints/app/agents/useOpenClaw'
import {
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
  const openClawEnabled =
    status?.status === 'running' && status.controlPlaneStatus === 'connected'
  const { agents: openClawAgents, loading: openClawAgentsLoading } =
    useOpenClawAgents(openClawEnabled)
  const { agents: harnessAgents, loading: harnessAgentsLoading } =
    useHarnessAgents()
  const visibleOpenClawAgents = openClawEnabled ? openClawAgents : []
  const agents = [...visibleOpenClawAgents, ...harnessAgents]

  return (
    <Outlet
      context={
        {
          agents,
          agentsLoading:
            harnessAgentsLoading ||
            statusLoading ||
            (openClawEnabled && openClawAgentsLoading),
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
