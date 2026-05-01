import { type FC, useMemo } from 'react'
import type {
  HarnessAdapterDescriptor,
  HarnessAgent,
  HarnessAgentAdapter,
} from '@/entrypoints/app/agents/agent-harness-types'
import type { AgentAdapterHealth } from '@/entrypoints/app/agents/agent-row/agent-row.types'
import { orderAgentsByPinThenRecency } from '@/entrypoints/app/agents/agents-list-order'
import { AgentRailRow } from './AgentRailRow'

interface AgentRailProps {
  agents: HarnessAgent[]
  adapters: HarnessAdapterDescriptor[]
  activeAgentId: string
  onSelectAgent: (agent: HarnessAgent) => void
  onPinToggle: (agent: HarnessAgent, next: boolean) => void
}

/**
 * Left-column scrollable list of agents. The "Agents" label + back
 * button live in the shared top band above (so the rail header and
 * the chat header sit on a single aligned strip rather than as two
 * separately-sized headers per column). Sort matches `/agents`:
 * pinned-first → recency, so the rail doesn't reshuffle as turns
 * transition every 5 s.
 */
export const AgentRail: FC<AgentRailProps> = ({
  agents,
  adapters,
  activeAgentId,
  onSelectAgent,
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

  const ordered = useMemo(() => orderAgentsByPinThenRecency(agents), [agents])

  return (
    <aside className="hidden min-h-0 flex-col border-border/50 border-r bg-background/70 lg:flex">
      <div className="styled-scrollbar min-h-0 flex-1 space-y-1.5 overflow-y-auto px-3 py-3">
        {ordered.map((agent) => (
          <AgentRailRow
            key={agent.id}
            agent={agent}
            active={agent.id === activeAgentId}
            adapterHealth={adapterHealth.get(agent.adapter) ?? null}
            onSelect={() => onSelectAgent(agent)}
            onPinToggle={(next) => onPinToggle(agent, next)}
          />
        ))}
      </div>
    </aside>
  )
}
