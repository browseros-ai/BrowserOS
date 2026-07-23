import { useQueryClient } from '@tanstack/react-query'
import { useLiveSessions, useSessions } from '@/modules/api/audit.hooks'
import { useCancelSession } from '@/modules/api/cancel.hooks'
import { useFocusBrowserTab } from '@/modules/api/focus.hooks'
import type { LiveSessionCardRecord } from '@/screens/cockpit/cockpit.helpers'
import { AgentRunningCard } from './AgentRunningCard'

interface RunningGridProps {
  sessions: LiveSessionCardRecord[]
}

/** Renders one card and one set of controls per connected live session. */
export function RunningGrid({ sessions }: RunningGridProps) {
  const queryClient = useQueryClient()
  const focus = useFocusBrowserTab()
  const cancel = useCancelSession()

  if (sessions.length === 0) return null

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

  return (
    <section className="space-y-4">
      <header className="flex items-baseline gap-3">
        <h2 className="font-semibold text-ink text-lg">Running now</h2>
        <span className="inline-flex items-center gap-1.5 font-mono text-[11px] text-accent uppercase tracking-[0.08em]">
          <span
            aria-hidden
            className="inline-block size-1.5 animate-[pulse-dot_1.4s_ease-in-out_infinite] rounded-full bg-accent shadow-[0_0_8px_hsl(221_90%_55%/0.5)]"
          />
          {sessions.length} live
        </span>
      </header>
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
        {sessions.map((session) => (
          <AgentRunningCard
            key={session.sessionId}
            session={session}
            onWatch={session.selectedTab ? () => onWatch(session) : undefined}
            onStop={() => onStop(session.sessionId)}
            isFocusPending={
              pendingBrowserTabId === session.selectedTab?.browserTabId
            }
            isCancelPending={cancelPendingSessionId === session.sessionId}
          />
        ))}
      </div>
    </section>
  )
}
