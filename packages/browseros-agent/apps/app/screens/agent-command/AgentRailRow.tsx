import type { FC } from 'react'
import { AgentSummaryChips } from '@/components/agents/agent-row/AgentSummaryChips'
import { AgentTile } from '@/components/agents/agent-row/AgentTile'
import type { AgentAdapterHealth } from '@/components/agents/agent-row/agent-row.types'
import { PinToggle } from '@/components/agents/agent-row/PinToggle'
import { Badge } from '@/components/ui/badge'
import { cn } from '@/lib/utils'
import type { HarnessAgent } from '@/modules/agents/agent-harness-types'

export interface AgentRailRowProps {
  agent: HarnessAgent
  active: boolean
  adapterHealth: AgentAdapterHealth | null
  onSelect: () => void
  onPinToggle: (next: boolean) => void
}

/**
 * Compact rail row for the chat-screen sidebar. Slims `<AgentRowCard>`
 * down to the essentials that fit a ~280 px rail: tile + name + status
 * badge + pin star, with the adapter / model / reasoning chips on a
 * second line. Token totals, sparkline, last-message preview all stay
 * on the `/agents` page where rows are full-width.
 */
export const AgentRailRow: FC<AgentRailRowProps> = ({
  agent,
  active,
  adapterHealth,
  onSelect,
  onPinToggle,
}) => {
  const status = agent.status ?? 'unknown'
  const lastUsedAt = agent.lastUsedAt ?? null
  const pinned = agent.pinned ?? false
  return (
    <button
      type="button"
      onClick={onSelect}
      className={cn(
        'group w-full rounded-2xl border px-3 py-3 text-left transition-colors',
        active
          ? 'border-[var(--accent-orange)]/30 bg-[var(--accent-orange)]/8'
          : 'border-transparent bg-transparent hover:border-border/60 hover:bg-card',
      )}
    >
      <div className="flex min-w-0 items-start gap-3">
        <AgentTile
          adapter={agent.adapter}
          status={status}
          lastUsedAt={lastUsedAt}
        />
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5">
            <span className="truncate font-semibold text-[14px] leading-5">
              {agent.name}
            </span>
            {status === 'working' && (
              <Badge
                variant="secondary"
                className="h-5 bg-amber-50 px-1.5 text-[10px] text-amber-900 hover:bg-amber-50"
              >
                Working
              </Badge>
            )}
            {status === 'asleep' && (
              <Badge
                variant="outline"
                className="h-5 px-1.5 text-[10px] text-muted-foreground"
              >
                Asleep
              </Badge>
            )}
            {status === 'error' && (
              <Badge variant="destructive" className="h-5 px-1.5 text-[10px]">
                Attention
              </Badge>
            )}
            <div className="ml-auto">
              <PinToggle pinned={pinned} onToggle={onPinToggle} />
            </div>
          </div>
          <AgentSummaryChips
            adapter={agent.adapter}
            modelLabel={agent.modelId ?? null}
            reasoningEffort={agent.reasoningEffort ?? null}
            adapterHealth={adapterHealth}
          />
        </div>
      </div>
    </button>
  )
}
