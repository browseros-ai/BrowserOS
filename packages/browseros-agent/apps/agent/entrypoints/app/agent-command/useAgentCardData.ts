import {
  type AgentEntry,
  getModelDisplayName,
  type OpenClawStatus,
} from '@/entrypoints/app/agents/useOpenClaw'
import type { AgentCardData } from '@/lib/agent-conversations/types'
import type { AgentOverview } from './useAgentDashboard'

function getAgentStatusTone(
  status: OpenClawStatus['status'] | undefined,
): AgentCardData['status'] {
  if (status === 'error') return 'error'
  if (status === 'starting') return 'working'
  return 'idle'
}

/**
 * Build agent card display data by merging the raw agent entries from
 * the gateway with enriched overview data from the dashboard API.
 *
 * Pure function — no hooks, no IndexedDB, no async.
 */
export function buildAgentCardData(
  agents: AgentEntry[],
  status: OpenClawStatus['status'] | undefined,
  dashboard: AgentOverview[] | undefined,
): AgentCardData[] {
  return agents.map((agent) => {
    const overview = dashboard?.find((d) => d.agentId === agent.agentId)

    return {
      agentId: agent.agentId,
      name: agent.name,
      model: getModelDisplayName(agent.model),
      status: getAgentStatusTone(status),
      lastMessage: overview?.latestMessage?.slice(0, 200) ?? undefined,
      lastMessageTimestamp: overview?.latestMessageAt ?? undefined,
      activitySummary: overview?.activitySummary ?? undefined,
      costUsd: overview?.totalCostUsd ?? undefined,
    }
  })
}
