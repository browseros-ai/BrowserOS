import { ArrowLeft } from 'lucide-react'
import { type FC, useMemo } from 'react'
import { Button } from '@/components/ui/button'
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
  onGoHome: () => void
}

/**
 * Left sidebar on `/agents/:agentId`. Header strip with the back-button
 * sits flush above the scrollable list. Sort uses the same pin-first →
 * recency comparator the `/agents` page uses, not the `/home`
 * active-first one — chat is index-shaped: shuffling rows every 5 s as
 * turns transition would be jarring while the user is reading.
 */
export const AgentRail: FC<AgentRailProps> = ({
  agents,
  adapters,
  activeAgentId,
  onSelectAgent,
  onPinToggle,
  onGoHome,
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
      <div className="flex h-14 shrink-0 items-center border-border/50 border-b px-4">
        <div className="flex min-w-0 items-center gap-3">
          <Button
            variant="ghost"
            size="icon"
            onClick={onGoHome}
            className="size-8 rounded-xl"
            title="Back to home"
          >
            <ArrowLeft className="size-4" />
          </Button>
          <div className="truncate font-semibold text-[15px] leading-5">
            Agents
          </div>
        </div>
      </div>
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
