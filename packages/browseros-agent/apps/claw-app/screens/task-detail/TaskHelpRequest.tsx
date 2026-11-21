import type { SessionSummary } from '@browseros/claw-api'
import { InControlBar } from '@/components/cockpit/InControlBar'
import { NeedsYouBanner } from '@/components/cockpit/NeedsYouBanner'
import { useLiveSessionState } from '@/modules/api/audit.hooks'
import { sessionsToLiveCards } from '@/screens/cockpit/cockpit.helpers'
import { useHelpTakeover } from '@/screens/cockpit/cockpit.hooks'

/** Reuses Home's help flow against the same live snapshot while viewing a recorded task. */
export function TaskHelpRequest({ session }: { session: SessionSummary }) {
  const live = useLiveSessionState(
    session.status === 'live' ? session.sessionId : undefined,
  )
  const sessions =
    session.status === 'live' && live?.helpRequest
      ? sessionsToLiveCards([{ ...session, live }])
      : []
  const takeover = useHelpTakeover(sessions)

  return (
    <>
      <NeedsYouBanner
        sessions={sessions}
        onTakeOver={takeover.takeOver}
        onStop={takeover.stop}
        pendingTakeOverSessionId={takeover.pendingTakeOverSessionId}
        cancelPendingSessionId={takeover.stopPendingSessionId}
      />
      {takeover.inControlSession ? (
        <InControlBar
          session={takeover.inControlSession}
          onHandBack={takeover.handBack}
          onCancel={takeover.cancelControl}
          isHandingBack={takeover.isHandingBack}
        />
      ) : null}
    </>
  )
}
