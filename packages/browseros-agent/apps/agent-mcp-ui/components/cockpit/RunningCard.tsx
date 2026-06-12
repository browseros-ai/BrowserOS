import {
  Check,
  ExternalLink,
  History,
  RefreshCw,
  Square,
  X,
} from 'lucide-react'
import { Card } from '@/components/ui/card'
import { isActiveStatus, isEndedStatus } from '@/lib/status'
import type { AgentRow } from '@/modules/api/agents.hooks'
import { MiniScreencast } from './MiniScreencast'
import { StatusBadge } from './StatusBadge'

interface RunningCardProps {
  agent: AgentRow
  onWatch?: () => void
  onStop?: () => void
}

/**
 * Uniform agent run card. Mini-screencast on top, label + status badge
 * inline, the site host in mono, a truncated 2-line task description,
 * a live status one-liner with a spinner / check / x icon, and a
 * footer with Watch + Stop (running) or Replay (ended). Hover lifts
 * the shadow and strengthens the border so the card feels clickable.
 */
export function RunningCard({ agent, onWatch, onStop }: RunningCardProps) {
  const active = isActiveStatus(agent.status)
  const ended = isEndedStatus(agent.status)

  return (
    <Card className="group flex cursor-pointer flex-col overflow-hidden border-border-2 p-0 transition hover:border-border-strong hover:shadow-card">
      <MiniScreencast site={agent.site} live={active} />
      <div className="flex flex-1 flex-col gap-2 p-3.5">
        <div className="flex items-center gap-2">
          <span className="min-w-0 flex-1 truncate font-bold text-[13.5px]">
            {agent.label}
          </span>
          <StatusBadge status={agent.status} />
        </div>
        <code className="truncate font-mono text-[11px] text-ink-3">
          {agent.site}
        </code>
        <p className="line-clamp-2 min-h-9 text-[12.5px] text-ink-2 leading-snug">
          {agent.task}
        </p>
        <div className="mt-auto flex items-center gap-1.5 text-[11.5px] text-ink-2">
          {active ? (
            <RefreshCw className="size-3 shrink-0 animate-spin text-accent" />
          ) : agent.status === 'stopped' ? (
            <X className="size-3 shrink-0 text-ink-3" />
          ) : (
            <Check className="size-3 shrink-0 text-green" />
          )}
          <span className="min-w-0 flex-1 truncate font-mono">
            {active
              ? agent.liveLine
              : agent.status === 'stopped'
                ? 'Stopped by you'
                : 'Completed'}
          </span>
        </div>
        <div className="flex gap-2 border-border border-t pt-2.5">
          <button
            type="button"
            onClick={ended ? onWatch : onWatch}
            className="inline-flex flex-1 items-center justify-center gap-1.5 rounded-md px-2 py-1.5 font-semibold text-[12.5px] text-ink-2 transition hover:bg-bg-sunken hover:text-ink"
          >
            {ended ? (
              <>
                <History className="size-3.5" /> Replay
              </>
            ) : (
              <>
                <ExternalLink className="size-3.5" /> Watch
              </>
            )}
          </button>
          {active && (
            <button
              type="button"
              onClick={onStop}
              className="inline-flex items-center justify-center gap-1.5 rounded-md bg-bg-sunken px-2.5 py-1.5 font-semibold text-[12.5px] text-ink-2 transition hover:bg-card-tint hover:text-ink"
            >
              <Square className="size-3" /> Stop
            </button>
          )}
        </div>
      </div>
    </Card>
  )
}
