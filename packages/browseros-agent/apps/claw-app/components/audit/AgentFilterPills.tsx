import { cn } from '@/lib/utils'
import type { AgentChip } from '@/screens/audit/audit.helpers'

interface AgentFilterPillsProps {
  chips: AgentChip[]
  selectedAgentId: string | null
  onSelect: (agentId: string | null) => void
}

/**
 * Per-agent filter pills above the audit list. Click a chip to filter
 * the list to that agent; click again (or the All pill) to clear.
 */
export function AgentFilterPills({
  chips,
  selectedAgentId,
  onSelect,
}: AgentFilterPillsProps) {
  if (chips.length === 0) return null
  const allActive = selectedAgentId === null
  return (
    <div className="flex flex-wrap items-center gap-2">
      <button
        type="button"
        onClick={() => onSelect(null)}
        className={cn(
          'inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 font-semibold text-[12px] transition',
          allActive
            ? 'border-ink bg-ink text-card'
            : 'border-border-2 bg-card text-ink-2 hover:bg-bg-sunken',
        )}
      >
        All
      </button>
      {chips.map((chip) => {
        const isActive = chip.agentId === selectedAgentId
        return (
          <button
            key={chip.agentId}
            type="button"
            onClick={() => onSelect(isActive ? null : chip.agentId)}
            className={cn(
              'inline-flex items-center gap-2 rounded-full border px-2.5 py-1 font-semibold text-[12px] transition',
              isActive
                ? 'border-ink bg-ink text-card'
                : 'border-border-2 bg-card text-ink-2 hover:bg-bg-sunken',
            )}
          >
            <span
              aria-hidden
              className="size-2 rounded-full"
              style={{ backgroundColor: chip.color }}
            />
            {chip.agentLabel}
            <span
              className={cn(
                'rounded-full px-1.5 py-0.5 font-mono text-[10.5px]',
                isActive ? 'bg-card/20 text-card' : 'bg-bg-sunken text-ink-3',
              )}
            >
              {chip.count}
            </span>
          </button>
        )
      })}
    </div>
  )
}
