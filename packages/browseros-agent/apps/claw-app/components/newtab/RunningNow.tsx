import type { CockpitStats } from '@browseros/claw-api'
import { useQueryClient } from '@tanstack/react-query'
import { ArrowRight } from 'lucide-react'
import { NavLink } from 'react-router'
import { AgentRunningCard } from '@/components/cockpit/AgentRunningCard'
import { cn } from '@/lib/utils'
import { useLiveSessions, useSessions } from '@/modules/api/audit.hooks'
import { useCancelSession } from '@/modules/api/cancel.hooks'
import { useFocusBrowserTab } from '@/modules/api/focus.hooks'
import type { LiveSessionCardRecord } from '@/screens/cockpit/cockpit.helpers'
import { StatGlanceInline } from './StatGlance'

interface RunningNowProps {
  sessions: LiveSessionCardRecord[]
  lead: LiveSessionCardRecord | null
  rest: LiveSessionCardRecord[]
  stats: CockpitStats | undefined
  statsPending: boolean
}

/**
 * Promoted live monitor: the most-active session as a large lead card beside
 * a compact rail of the remaining sessions, with the live count and the
 * compact saved stat in the header.
 */
export function RunningNow({
  sessions,
  lead,
  rest,
  stats,
  statsPending,
}: RunningNowProps) {
  const queryClient = useQueryClient()
  const focus = useFocusBrowserTab()
  const cancel = useCancelSession()

  if (lead === null) return null
  const now = Date.now()

  const onWatch = (session: LiveSessionCardRecord) => {
    const browserTabId = session.selectedTab?.browserTabId
    if (browserTabId === undefined) return
    focus.mutate(
      { browserTabId },
      {
        onError: (err) => {
          // eslint-disable-next-line no-console
          console.warn('focus browser tab failed', {
            sessionId: session.sessionId,
            browserTabId,
            err,
          })
        },
      },
    )
  }
  const onStop = (sessionId: string) => {
    cancel.mutate(
      { sessionId },
      {
        onSuccess: () => {
          void queryClient.invalidateQueries({
            queryKey: useLiveSessions.getKey(),
          })
          void queryClient.invalidateQueries({
            queryKey: useSessions.getKey(),
          })
        },
        onError: (err) => {
          // eslint-disable-next-line no-console
          console.warn('cancel session failed', { sessionId, err })
        },
      },
    )
  }
  const pendingBrowserTabId =
    focus.isPending && focus.variables
      ? focus.variables.browserTabId
      : undefined
  const cancelPendingSessionId =
    cancel.isPending && cancel.variables
      ? cancel.variables.sessionId
      : undefined

  const cardProps = (session: LiveSessionCardRecord) => ({
    session,
    now,
    onWatch: session.selectedTab ? () => onWatch(session) : undefined,
    onStop: () => onStop(session.sessionId),
    isFocusPending: pendingBrowserTabId === session.selectedTab?.browserTabId,
    isCancelPending: cancelPendingSessionId === session.sessionId,
  })

  const hasRest = rest.length > 0

  return (
    <section className="space-y-4">
      <RunningNowHeader
        liveCount={sessions.length}
        stats={stats}
        statsPending={statsPending}
      />
      <div className="grid grid-cols-1 gap-3 lg:grid-cols-3">
        <div className={cn(hasRest ? 'lg:col-span-2' : 'lg:col-span-3')}>
          <AgentRunningCard variant="lead" {...cardProps(lead)} />
        </div>
        {hasRest && (
          <div className="flex gap-3 overflow-x-auto pb-1 lg:flex-col lg:overflow-visible lg:pb-0">
            {rest.map((session) => (
              <div
                key={session.sessionId}
                className="w-[248px] shrink-0 snap-start lg:w-auto lg:shrink"
              >
                <AgentRunningCard variant="compact" {...cardProps(session)} />
              </div>
            ))}
          </div>
        )}
      </div>
    </section>
  )
}

interface RunningNowHeaderProps {
  liveCount: number
  stats: CockpitStats | undefined
  statsPending: boolean
}

function RunningNowHeader({
  liveCount,
  stats,
  statsPending,
}: RunningNowHeaderProps) {
  return (
    <header className="flex flex-wrap items-center gap-x-3 gap-y-1">
      <h2 className="font-semibold text-ink text-lg">Running now</h2>
      <span className="inline-flex items-center gap-1.5 text-accent text-sm">
        <span
          aria-hidden
          className="inline-block size-1.5 animate-pulse-dot rounded-full bg-accent shadow-[0_0_8px_hsl(221_90%_55%/0.5)]"
        />
        {liveCount} live
      </span>
      <div className="ml-auto flex items-center gap-3">
        <StatGlanceInline stats={stats} pending={statsPending} />
        <NavLink
          to="/audit"
          className="group inline-flex items-center gap-1 text-ink-3 text-sm transition-colors hover:text-ink"
        >
          stats
          <ArrowRight className="size-3.5 transition-transform group-hover:translate-x-0.5" />
        </NavLink>
      </div>
    </header>
  )
}
