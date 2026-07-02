import { ExternalLink, RefreshCw, Square } from 'lucide-react'
import { AgentDot } from '@/components/audit/AgentDot'
import type { AgentActivityRecord } from '@/screens/cockpit/cockpit.helpers'
import { formatToolTrail, siteOf } from '@/screens/cockpit/cockpit.helpers'
import { MiniScreencast } from './MiniScreencast'
import { TabCountChip } from './TabCountChip'

interface AgentRunningCardProps {
  agent: AgentActivityRecord
  onWatch?: () => void
  onStop?: () => void
  /** When the focus mutation is in flight for this card. */
  isFocusPending?: boolean
  /** When the cancel mutation is in flight for this card. */
  isCancelPending?: boolean
}

/**
 * One card per LIVE agent in the "Running now" strip. Uses the
 * same split-zone language as the editorial lead-story tile in
 * Recent activity: the current focus screencast dominates the top,
 * a dark caption block at the bottom carries the agent identity,
 * the current tool, and Watch / Stop actions as inline chips.
 *
 * LIVE indicator uses the app's accent orange with a pulsing dot,
 * not a green pill; the whole app has one accent color.
 */
export function AgentRunningCard({
  agent,
  onWatch,
  onStop,
  isFocusPending,
  isCancelPending,
}: AgentRunningCardProps) {
  const focus = agent.currentFocus
  const active = agent.status === 'active'
  const trail = formatToolTrail(agent.recentTools)
  return (
    <div
      data-agent-card
      className="group relative flex h-[300px] flex-col overflow-hidden rounded-2xl border border-border-2 bg-bg-sunken transition-[border-color] duration-150 hover:border-accent/40"
    >
      <div className="relative flex-1 overflow-hidden">
        <MiniScreencast
          site={siteOf(focus.url)}
          live={active}
          screencast={focus.screencast}
        />
        <div className="absolute top-3 right-3">
          <TabCountChip tabs={agent.tabs} focusTargetId={focus.targetId} />
        </div>
      </div>
      <div className="flex flex-col gap-1.5 bg-ink px-4 py-3 text-white">
        <div className="flex items-center gap-3 font-mono text-[10.5px] text-white/80 uppercase tracking-[0.08em]">
          <span className="inline-flex items-center gap-1.5">
            <AgentDot slug={agent.slug} />
            <span className="text-white">{agent.agentLabel}</span>
          </span>
          {active && (
            <span className="inline-flex items-center gap-1.5 text-accent">
              <span
                aria-hidden
                className="inline-block size-1.5 animate-[pulse-dot_1.4s_ease-in-out_infinite] rounded-full bg-accent shadow-[0_0_8px_hsl(19_89%_56%/0.7)]"
              />
              LIVE
            </span>
          )}
        </div>
        <h3 className="truncate font-semibold text-[14px] text-white leading-tight">
          {focus.title || siteOf(focus.url)}
        </h3>
        <p className="truncate font-mono text-[11px] text-white/60">
          {trail || siteOf(focus.url)}
        </p>
        <div className="mt-1 flex items-center gap-2 border-white/10 border-t pt-2">
          <button
            type="button"
            onClick={onWatch}
            disabled={isFocusPending}
            className="inline-flex flex-1 items-center justify-center gap-1.5 rounded-md bg-white/10 px-2 py-1.5 font-mono text-[10.5px] text-white/90 uppercase tracking-[0.08em] transition hover:bg-white/20 disabled:cursor-not-allowed disabled:opacity-60"
          >
            {isFocusPending ? (
              <RefreshCw className="size-3 animate-spin" />
            ) : (
              <ExternalLink className="size-3" />
            )}
            Watch
          </button>
          {active && onStop && (
            <button
              type="button"
              onClick={onStop}
              disabled={isCancelPending}
              aria-label={isCancelPending ? 'Cancelling agent' : 'Stop agent'}
              // min-w reserves enough width for the longer
              // 'Cancelling' label so swapping in the pending state
              // does not push the Watch button around.
              className="inline-flex min-w-[92px] items-center justify-center gap-1.5 rounded-md bg-white/10 px-2 py-1.5 font-mono text-[10.5px] text-white/90 uppercase tracking-[0.08em] transition hover:bg-red-500/30 hover:text-red-100 disabled:cursor-not-allowed disabled:opacity-60"
            >
              {isCancelPending ? (
                <>
                  <RefreshCw className="size-3 animate-spin" /> Cancelling
                </>
              ) : (
                <>
                  <Square className="size-3" /> Stop
                </>
              )}
            </button>
          )}
        </div>
      </div>
    </div>
  )
}
