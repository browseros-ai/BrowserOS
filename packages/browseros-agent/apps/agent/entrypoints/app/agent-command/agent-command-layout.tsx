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
  // Dual-created OpenClaw agents appear in both `/claw/agents` (gateway
  // record) and `/agents` (harness record) under the same id. Prefer the
  // harness entry so the chat panel can route through the harness path
  // and the rail doesn't show duplicates.
  const harnessAgentIds = new Set(harnessAgents.map((entry) => entry.agentId))
  const dedupedOpenClawAgents = visibleOpenClawAgents.filter(
    (entry) => !harnessAgentIds.has(entry.agentId),
  )
  const agents = [...dedupedOpenClawAgents, ...harnessAgents]

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
