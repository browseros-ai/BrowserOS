import { Loader2 } from 'lucide-react'
import { type FC, useMemo } from 'react'
import { AgentRowCard } from './AgentRowCard'
import { AgentsEmptyState } from './AgentsEmptyState'
import type {
  HarnessAdapterDescriptor,
  HarnessAgent,
  HarnessAgentAdapter,
} from './agent-harness-types'
import type {
  AgentAdapterHealth,
  AgentRowData,
} from './agent-row/agent-row.types'
import { compareAgentsByPinThenRecency } from './agents-list-order'
import type { AgentListItem } from './agents-page-types'
import type { AgentLiveness } from './LivenessDot'

interface AgentListProps {
  agents: AgentListItem[]
  /** Optional per-agent activity metadata, keyed by `agentId`. */
  activity?: Record<
    string,
    { status: AgentLiveness; lastUsedAt: number | null }
  >
  /** Lookup table from harness id → enriched agent record. */
  harnessAgentLookup?: Map<string, HarnessAgent>
  /** Adapter catalog (carries per-adapter health). */
  adapters: HarnessAdapterDescriptor[]
  loading: boolean
  deletingAgentKey: string | null
  onCreateAgent: () => void
  onDeleteAgent: (agent: AgentListItem) => void
  onPinToggle: (agent: AgentListItem, next: boolean) => void
}

export const AgentList: FC<AgentListProps> = ({
  agents,
  activity,
  harnessAgentLookup,
  adapters,
  loading,
  deletingAgentKey,
  onCreateAgent,
  onDeleteAgent,
  onPinToggle,
}) => {
  const adapterHealth = useMemo(() => {
    const map = new Map<HarnessAgentAdapter, AgentAdapterHealth>()
    for (const adapter of adapters) {
      if (adapter.health) {
        map.set(adapter.id, {
          healthy: adapter.health.healthy,
          reason: adapter.health.reason,
        })
      }
    }
    return map
  }, [adapters])

  const ordered = useMemo(() => {
    const withMeta = agents.map((agent) => {
      const harness = harnessAgentLookup?.get(agent.agentId)
      return {
        agent,
        id: agent.agentId,
        pinned: harness?.pinned ?? false,
        lastUsedAt: activity?.[agent.agentId]?.lastUsedAt ?? null,
      }
    })
    return withMeta
      .sort(compareAgentsByPinThenRecency)
      .map((entry) => entry.agent)
  }, [activity, agents, harnessAgentLookup])

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
        const adapter: HarnessAgentAdapter | 'unknown' =
          harness?.adapter ?? inferAdapterFromLabel(agent.runtimeLabel)
        const data = buildRowData({
          agent,
          adapter,
          harness,
          activity: activity?.[agent.agentId],
          adapterHealth:
            adapterHealth.get(adapter as HarnessAgentAdapter) ?? null,
        })
        return (
          <AgentRowCard
            key={agent.key}
            data={data}
            deleting={deletingAgentKey === agent.key}
            onDelete={onDeleteAgent}
            onPinToggle={onPinToggle}
          />
        )
      })}
    </div>
  )
}

function inferAdapterFromLabel(label: string): HarnessAgentAdapter | 'unknown' {
  const lower = label?.toLowerCase()
  if (lower === 'claude code') return 'claude'
  if (lower === 'codex') return 'codex'
  if (lower === 'openclaw') return 'openclaw'
  return 'unknown'
}

const ZERO_BUCKETS = (): number[] => Array.from({ length: 14 }, () => 0)

function buildRowData(input: {
  agent: AgentListItem
  adapter: HarnessAgentAdapter | 'unknown'
  harness: HarnessAgent | undefined
  activity: { status: AgentLiveness; lastUsedAt: number | null } | undefined
  adapterHealth: AgentAdapterHealth | null
}): AgentRowData {
  const { agent, adapter, harness, activity, adapterHealth } = input
  return {
    agent,
    adapter,
    modelLabel: deriveModelLabel(agent, harness),
    reasoningEffort: harness?.reasoningEffort ?? null,
    status: activity?.status ?? 'unknown',
    lastUsedAt: activity?.lastUsedAt ?? harness?.lastUsedAt ?? null,
    pinned: harness?.pinned ?? false,
    cwd: harness?.cwd ?? null,
    lastUserMessage: harness?.lastUserMessage ?? null,
    tokens: harness?.tokens ?? null,
    turnsByDay: harness?.turnsByDay ?? ZERO_BUCKETS(),
    failedByDay: harness?.failedByDay ?? ZERO_BUCKETS(),
    lastError: harness?.lastError ?? null,
    lastErrorAt: harness?.lastErrorAt ?? null,
    activeTurnId: harness?.activeTurnId ?? null,
    adapterHealth,
  }
}

function deriveModelLabel(
  agent: AgentListItem,
  harness: HarnessAgent | undefined,
): string | null {
  // Prefer the agent rail's modelLabel when meaningful; harness's
  // modelId is a stable identifier but the rail's `modelLabel`
  // already maps to a friendly display string.
  if (agent.modelLabel && agent.modelLabel !== 'default') {
    return agent.modelLabel
  }
  return harness?.modelId ?? null
}
