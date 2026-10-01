import { useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { toast } from 'sonner'
import { useLiveSessions, useSessions } from '@/modules/api/audit.hooks'
import { useCancelSession } from '@/modules/api/cancel.hooks'
import { useFocusBrowserTab } from '@/modules/api/focus.hooks'
import { useResolveHelp } from '@/modules/api/help.hooks'
import type { LiveSessionCardRecord } from './cockpit.helpers'

export interface HelpTakeover {
  /**
   * The session the human is currently in control of, resolved from the live
   * snapshot so the bar closes on its own when the request is handed back,
   * times out, or the session ends. Null when nobody has taken over.
   */
  inControlSession: LiveSessionCardRecord | null
  /** Foreground the agent's blocked tab and open the in-control bar. */
  takeOver: (sessionId: string, browserTabId: number) => void
  /** Signal the waiting agent to resume with an optional note, then close the bar. */
  handBack: (sessionId: string, note?: string) => void
  /** Close the in-control bar without resuming; the agent keeps waiting. */
  cancelControl: () => void
  /** Terminally stop a session from the banner (the card has its own Stop). */
  stop: (sessionId: string) => void
  pendingTakeOverSessionId?: string
  stopPendingSessionId?: string
  isHandingBack: boolean
}

/**
 * Orchestrates the cockpit takeover flow shared by the needs-you banner, the
 * running cards, and the in-control bar: foreground the blocked tab, then hand
 * control back to the parked agent. Resuming invalidates the live snapshot so
 * the card drops its needs-you state immediately rather than at the next poll.
 */
export function useHelpTakeover(
  sessions: LiveSessionCardRecord[],
): HelpTakeover {
  const queryClient = useQueryClient()
  const focus = useFocusBrowserTab()
  const resolve = useResolveHelp()
  const cancel = useCancelSession()
  const [inControlSessionId, setInControlSessionId] = useState<string | null>(
    null,
  )
  const [pendingTakeOverSessionId, setPendingTakeOverSessionId] = useState<
    string | undefined
  >(undefined)

  const inControlSession =
    inControlSessionId === null
      ? null
      : (sessions.find(
          (session) =>
            session.sessionId === inControlSessionId &&
            session.helpRequest != null,
        ) ?? null)

  const takeOver = (sessionId: string, browserTabId: number) => {
    setInControlSessionId(sessionId)
    setPendingTakeOverSessionId(sessionId)
    focus.mutate(
      { browserTabId },
      {
        onSettled: () => setPendingTakeOverSessionId(undefined),
        onError: (err) => {
          toast.error('Could not bring the agent tab forward')
          // eslint-disable-next-line no-console
          console.warn('focus browser tab failed', { browserTabId, err })
        },
      },
    )
  }

  const handBack = (sessionId: string, note?: string) => {
    const trimmed = note?.trim()
    resolve.mutate(
      { sessionId, note: trimmed || undefined },
      {
        onSuccess: (result) => {
          setInControlSessionId(null)
          void queryClient.invalidateQueries({
            queryKey: useLiveSessions.getKey(),
          })
          if (result.resolved) toast.success('Handed back to the agent')
          else toast('The agent already stopped waiting')
        },
        onError: (err) => {
          toast.error('Could not hand back to the agent')
          // eslint-disable-next-line no-console
          console.warn('resolve help failed', { sessionId, err })
        },
      },
    )
  }

  const cancelControl = () => setInControlSessionId(null)

  const stop = (sessionId: string) => {
    cancel.mutate(
      { sessionId },
      {
        onSuccess: () => {
          if (inControlSessionId === sessionId) setInControlSessionId(null)
          void queryClient.invalidateQueries({
            queryKey: useLiveSessions.getKey(),
          })
          void queryClient.invalidateQueries({ queryKey: useSessions.getKey() })
        },
        onError: (err) => {
          toast.error('Could not stop the agent')
          // eslint-disable-next-line no-console
          console.warn('cancel session failed', { sessionId, err })
        },
      },
    )
  }

  return {
    inControlSession,
    takeOver,
    handBack,
    cancelControl,
    stop,
    pendingTakeOverSessionId,
    stopPendingSessionId:
      cancel.isPending && cancel.variables
        ? cancel.variables.sessionId
        : undefined,
    isHandingBack: resolve.isPending,
  }
}
