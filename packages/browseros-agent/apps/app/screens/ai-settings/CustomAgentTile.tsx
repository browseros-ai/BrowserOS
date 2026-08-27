import { Plus } from 'lucide-react'
import type { FC } from 'react'
import { BRAND_MARKS } from '@/components/agents/agent-brand-marks'
import { POPULAR_ACP_AGENTS } from './popular-acp-agents'

export interface CustomAgentTileProps {
  onAdd: () => void
}

/**
 * The custom-agent entry, given more room than the fixed-adapter tiles beside
 * it. "Custom ACP agent" says what it is but not what it gets you, so the
 * tile shows the marks of the agents the picker actually offers: the point is
 * that any ACP agent works, and a plus sign alone does not carry that.
 */
export const CustomAgentTile: FC<CustomAgentTileProps> = ({ onAdd }) => {
  const marks = POPULAR_ACP_AGENTS.map((agent) => ({
    id: agent.id,
    label: agent.label,
    Mark: BRAND_MARKS[agent.id],
  })).filter((entry) => Boolean(entry.Mark))

  return (
    <button
      type="button"
      onClick={onAdd}
      aria-label="Add Custom ACP agent"
      className="group col-span-full flex w-full items-center gap-4 rounded-lg border border-[var(--accent-orange)]/40 border-dashed bg-card p-4 text-left transition-all hover:border-[var(--accent-orange)] hover:bg-[var(--accent-orange)]/5"
    >
      <span className="flex size-10 shrink-0 items-center justify-center rounded-lg bg-[var(--accent-orange)]/10 text-[var(--accent-orange)]">
        <Plus className="size-6" />
      </span>

      <span className="min-w-0 flex-1">
        <span className="block font-medium text-foreground text-sm">
          Custom ACP agent
        </span>
        <span className="block text-muted-foreground text-xs">
          Connect opencode, Hermes, OpenClaw, pi, or any other ACP compatible
          agent
        </span>
        <span className="mt-2 flex flex-wrap items-center gap-1.5">
          {marks.map(({ id, label, Mark }) => (
            <span
              key={id}
              title={label}
              className="flex size-7 items-center justify-center overflow-hidden rounded-md border border-border bg-background"
            >
              <Mark className="size-4" />
            </span>
          ))}
        </span>
      </span>

      <span
        aria-hidden
        className="shrink-0 rounded-md border border-border bg-secondary px-3 py-1.5 font-semibold text-foreground text-xs transition-colors group-hover:border-[var(--accent-orange)] group-hover:bg-[var(--accent-orange)] group-hover:text-white"
      >
        Add
      </span>
    </button>
  )
}
