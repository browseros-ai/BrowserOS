import { Check, ExternalLink, RefreshCw, Square } from 'lucide-react'
import { Card } from '@/components/ui/card'
import type { AgentActivityRecord } from '@/screens/cockpit/cockpit.helpers'
import { formatToolTrail, siteOf } from '@/screens/cockpit/cockpit.helpers'
import { MiniScreencast } from './MiniScreencast'
import { StatusBadge } from './StatusBadge'
import { TabCountChip } from './TabCountChip'

interface AgentRunningCardProps {
  agent: AgentActivityRecord
  onWatch?: () => void
  onStop?: () => void
  /** When the focus mutation is in flight for this card. */
  isFocusPending?: boolean
}

/**
 * One card per agent on the homepage running grid. The focus tab is
 * the agent's freshest `lastToolAt` and supplies the visible
 * surface: site host, task title, mini-screencast. The "N tabs" chip
 * in the header opens a popover with the full tab list when the
 * agent is driving more than one tab. Trail and action count are
 * merged across all of this agent's tabs so the card tells the
 * complete story of what the agent has been doing, not just the
 * current focus.
 */
export function AgentRunningCard({
  agent,
  onWatch,
  onStop,
  isFocusPending,
}: AgentRunningCardProps) {
  const focus = agent.currentFocus
  const active = agent.status === 'active'
  const trail = formatToolTrail(agent.recentTools)
  const liveLine = `${agent.lastToolName} - ${focus.title || siteOf(focus.url)}`

  return (
    <Card
      data-agent-card
      className="group flex cursor-pointer flex-col overflow-hidden border-border-2 border-l-4 p-0 transition hover:border-border-strong hover:shadow-card"
      style={{ borderLeftColor: agent.color }}
    >
      <MiniScreencast site={siteOf(focus.url)} live={active} />
      <div className="flex flex-1 flex-col gap-2 p-3.5">
        <div className="flex items-center gap-2">
          <span className="min-w-0 flex-1 truncate font-bold text-[13.5px]">
            {agent.agentLabel}
          </span>
          <TabCountChip tabs={agent.tabs} focusTargetId={focus.targetId} />
          <StatusBadge status={active ? 'running' : 'done'} />
        </div>
        <code className="truncate font-mono text-[11px] text-ink-3">
          {siteOf(focus.url)}
        </code>
        <p className="line-clamp-2 min-h-9 text-[12.5px] text-ink-2 leading-snug">
          {focus.title || siteOf(focus.url)}
        </p>
        <div className="mt-auto flex items-center gap-1.5 text-[11.5px] text-ink-2">
          {active ? (
            <RefreshCw className="size-3 shrink-0 animate-spin text-accent" />
          ) : (
            <Check className="size-3 shrink-0 text-green" />
          )}
          <span className="min-w-0 flex-1 truncate font-mono">
            {active ? liveLine : 'Completed'}
          </span>
          {agent.toolCount > 0 && (
            <span className="shrink-0 rounded-full bg-bg-sunken px-1.5 py-0.5 font-mono text-[10.5px] text-ink-2">
              {agent.toolCount} {agent.toolCount === 1 ? 'action' : 'actions'}
            </span>
          )}
        </div>
        {trail && (
          <code
            className="truncate font-mono text-[10.5px] text-ink-3"
            title={trail}
          >
            {trail}
          </code>
        )}
        <div className="flex gap-2 border-border border-t pt-2.5">
          <button
            type="button"
            onClick={onWatch}
            disabled={isFocusPending}
            className="inline-flex flex-1 items-center justify-center gap-1.5 rounded-md px-2 py-1.5 font-semibold text-[12.5px] text-ink-2 transition hover:bg-bg-sunken hover:text-ink disabled:cursor-not-allowed disabled:opacity-60"
          >
            {isFocusPending ? (
              <RefreshCw className="size-3.5 animate-spin" />
            ) : (
              <ExternalLink className="size-3.5" />
            )}{' '}
            Watch
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
