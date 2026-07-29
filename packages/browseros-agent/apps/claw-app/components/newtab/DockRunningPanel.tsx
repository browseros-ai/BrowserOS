import { ArrowRight } from 'lucide-react'
import { NavLink } from 'react-router'
import { AgentRunningCard } from '@/components/cockpit/AgentRunningCard'
import type { LiveSessionCardRecord } from '@/screens/cockpit/cockpit.helpers'
import type { StatsSlice } from '@/screens/cockpit/newtab.data'
import { DockRestRow } from './DockRestRow'
import { StatGlance } from './StatGlance'
import { useRunningControls } from './use-running-controls.hooks'

interface DockRunningPanelProps {
  sessions: LiveSessionCardRecord[]
  stats: StatsSlice
}

/**
 * The first-class watch view: a prominent live lead card plus a scrolling list
 * of compact rest rows. The newest run leads. The value stat rides in the
 * header so it is never buried behind the running cards.
 */
export function DockRunningPanel({ sessions, stats }: DockRunningPanelProps) {
  const controls = useRunningControls()
  const now = Date.now()
  const ordered = [...sessions].sort(
    (left, right) => right.startedAt - left.startedAt,
  )
  const lead = ordered[0]
  const rest = ordered.slice(1)

  if (!lead) return null

  return (
    <section className="mx-auto w-full max-w-4xl animate-fade-up">
      <header className="flex flex-wrap items-center gap-x-3 gap-y-1">
        <h2 className="font-semibold text-ink text-lg">Running now</h2>
        <span
          aria-live="polite"
          className="inline-flex items-center gap-1.5 text-accent-ink text-sm"
        >
          <span
            aria-hidden
            className="inline-block size-1.5 animate-pulse-dot rounded-full bg-accent shadow-[0_0_8px_hsl(221_90%_55%/0.5)]"
          />
          {sessions.length} running
        </span>
        <div className="ml-auto flex items-center gap-3">
          <StatGlance compact isPending={stats.isPending} stats={stats.data} />
          <NavLink
            className="group inline-flex items-center gap-1 text-ink-3 text-xs transition-colors hover:text-ink"
            to="/audit"
          >
            all activity
            <ArrowRight className="size-3 transition-transform group-hover:translate-x-0.5" />
          </NavLink>
        </div>
      </header>

      <div className="mt-3 grid gap-3 md:grid-cols-[minmax(0,1.4fr)_minmax(0,1fr)]">
        <AgentRunningCard
          isCancelPending={controls.cancelPendingSessionId === lead.sessionId}
          isFocusPending={
            controls.pendingBrowserTabId === lead.selectedTab?.browserTabId
          }
          now={now}
          onStop={() => controls.onStop(lead.sessionId)}
          onWatch={lead.selectedTab ? () => controls.onWatch(lead) : undefined}
          session={lead}
          size="lead"
        />
        {rest.length > 0 ? (
          <div className="flex max-h-[clamp(240px,34dvh,360px)] flex-col gap-2 overflow-y-auto pr-1">
            {rest.map((session) => (
              <DockRestRow
                isCancelPending={
                  controls.cancelPendingSessionId === session.sessionId
                }
                isFocusPending={
                  controls.pendingBrowserTabId ===
                  session.selectedTab?.browserTabId
                }
                key={session.sessionId}
                now={now}
                onStop={() => controls.onStop(session.sessionId)}
                onWatch={
                  session.selectedTab
                    ? () => controls.onWatch(session)
                    : undefined
                }
                session={session}
              />
            ))}
          </div>
        ) : null}
      </div>
    </section>
  )
}
