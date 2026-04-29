import { Loader2 } from 'lucide-react'
import { type FC, useMemo } from 'react'
import { AgentRowCard } from './AgentRowCard'
import { AgentsEmptyState } from './AgentsEmptyState'
import type { HarnessAgent, HarnessAgentAdapter } from './agent-harness-types'
import type { AgentListItem } from './agents-page-types'
import type { AgentLiveness } from './LivenessDot'

interface AgentListProps {
  agents: AgentListItem[]
  /**
   * Optional per-agent activity metadata. Keyed by `agentId`. Missing
   * entries fall back to status='unknown' / lastUsedAt=null and the
   * row renders an "unknown" dot. The server will populate this once
   * the activity tracker ships; the page works without it.
   */
  activity?: Record<
    string,
    { status: AgentLiveness; lastUsedAt: number | null }
  >
  /**
   * Lookup table from harness agent id → adapter + reasoning effort,
   * sourced from `useHarnessAgents`. Lets the row card render the
   * correct adapter icon and chips for harness agents (legacy
   * /claw/agents entries fall back to inferring from `runtimeLabel`).
   */
  harnessAgentLookup?: Map<string, HarnessAgent>
  loading: boolean
  deletingAgentKey: string | null
  onCreateAgent: () => void
  onDeleteAgent: (agent: AgentListItem) => void
}

export const AgentList: FC<AgentListProps> = ({
  agents,
  activity,
  harnessAgentLookup,
  loading,
  deletingAgentKey,
  onCreateAgent,
  onDeleteAgent,
}) => {
  // Sort by recency: most recently used first; never-used agents drop
  // to the bottom in id-stable order so the list doesn't reshuffle on
  // every refresh. The pinned exception is the gateway's `main` agent
  // when it's never been touched — keep it at the top so a fresh
  // install has an obvious starting point.
  const ordered = useMemo(() => {
    const withScore = agents.map((agent) => {
      const lastUsedAt = activity?.[agent.agentId]?.lastUsedAt ?? null
      return { agent, lastUsedAt }
    })
    return withScore
      .sort((a, b) => {
        const aPinned = a.agent.agentId === 'main' && a.lastUsedAt === null
        const bPinned = b.agent.agentId === 'main' && b.lastUsedAt === null
        if (aPinned && !bPinned) return -1
        if (!aPinned && bPinned) return 1
        const aValue = a.lastUsedAt ?? -Infinity
        const bValue = b.lastUsedAt ?? -Infinity
        if (aValue !== bValue) return bValue - aValue
        return a.agent.agentId.localeCompare(b.agent.agentId)
      })
      .map((entry) => entry.agent)
  }, [activity, agents])

  if (loading && agents.length === 0) {
    return (
      <div className="flex h-36 items-center justify-center rounded-xl border border-border border-dashed bg-card/50">
        <Loader2 className="size-5 animate-spin text-muted-foreground" />
      </div>
    )
  }

  if (agents.length === 0) {
    return <AgentsEmptyState onCreateAgent={onCreateAgent} />
  }

  return (
    <div className="grid gap-3">
      {ordered.map((agent) => {
        const harness = harnessAgentLookup?.get(agent.agentId)
        const adapter: HarnessAgentAdapter | undefined =
          harness?.adapter ?? inferAdapterFromLabel(agent.runtimeLabel)
        return (
          <AgentRowCard
            key={agent.key}
            agent={agent}
            status={activity?.[agent.agentId]?.status}
            lastUsedAt={activity?.[agent.agentId]?.lastUsedAt}
            adapter={adapter}
            reasoningEffort={harness?.reasoningEffort ?? null}
            onDelete={onDeleteAgent}
            deleting={deletingAgentKey === agent.key}
          />
        )
      })}
    </div>
  )
}

function inferAdapterFromLabel(label: string): HarnessAgentAdapter | undefined {
  const lower = label?.toLowerCase()
  if (lower === 'claude code') return 'claude'
  if (lower === 'codex') return 'codex'
  if (lower === 'openclaw') return 'openclaw'
  return undefined
}
