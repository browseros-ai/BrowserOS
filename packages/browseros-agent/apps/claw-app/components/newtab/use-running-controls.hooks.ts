import { useQueryClient } from '@tanstack/react-query'
import { useLiveSessions, useSessions } from '@/modules/api/audit.hooks'
import { useCancelSession } from '@/modules/api/cancel.hooks'
import { useFocusBrowserTab } from '@/modules/api/focus.hooks'
import type { LiveSessionCardRecord } from '@/screens/cockpit/cockpit.helpers'

export interface RunningControls {
  onWatch: (session: LiveSessionCardRecord) => void
  onStop: (sessionId: string) => void
  pendingBrowserTabId?: number
  cancelPendingSessionId?: string
}

/**
 * One Watch / Stop implementation shared by the lead card and the rest rows.
 * Stop invalidates the live-session and paginated-session caches by their
 * `getKey` so both the running panel and the audit history refresh.
 */
export function useRunningControls(): RunningControls {
  const queryClient = useQueryClient()
  const focus = useFocusBrowserTab()
  const cancel = useCancelSession()

  const onWatch = (session: LiveSessionCardRecord) => {
    const browserTabId = session.selectedTab?.browserTabId
    if (browserTabId === undefined) return
    focus.mutate(
      { browserTabId },
      {
        onError: (err) => {
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

  return { onWatch, onStop, pendingBrowserTabId, cancelPendingSessionId }
}
