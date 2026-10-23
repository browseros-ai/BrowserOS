import type { CockpitStats } from '@browseros/claw-api'
import { CockpitOnboarding } from '@/components/cockpit/CockpitOnboarding'
import { InControlBar } from '@/components/cockpit/InControlBar'
import { NeedsYouBanner } from '@/components/cockpit/NeedsYouBanner'
import {
  PromoSlot,
  type PromoSlotState,
  usePromoSlot,
} from '@/components/cockpit/PromoSlot'
import { RecentActivity } from '@/components/cockpit/RecentActivity'
import { RunningGrid } from '@/components/cockpit/RunningGrid'
import { TimeSavedCard } from '@/components/cockpit/TimeSavedCard'
import { isUserFacingHarness } from '@/components/harness/harness.types'
import { Page } from '@/components/layout/Page'
import { PageHeader } from '@/components/layout/PageHeader'
import { cn } from '@/lib/utils'
import { useSessions } from '@/modules/api/audit.hooks'
import { useCockpitStats } from '@/modules/api/cockpit.hooks'
import { useConnections } from '@/modules/api/connections.hooks'
import { useCockpitData } from './cockpit.data'
import { homeLede, partOfDay } from './cockpit.helpers'
import { useCurrentHour, useHelpTakeover } from './cockpit.hooks'
import { getOnboardingState } from './cockpit-onboarding.helpers'

const ONBOARDING_PROBE_LIMIT = 1

export function Cockpit() {
  const { sessions } = useCockpitData()
  const takeover = useHelpTakeover(sessions)
  const hour = useCurrentHour()
  const promo = usePromoSlot()

  // When no live session is connected, these probes decide which onboarding
  // shell to show. Their stable keys are shared with RecentActivity and MCP.
  const connections = useConnections({ refetchInterval: 5000 })
  const taskProbe = useSessions({
    variables: { limit: ONBOARDING_PROBE_LIMIT },
    // Scoped to the onboarding shells: poll every 4s while the
    // reader has no activity yet so the 'ready' handoff lands
    // within a few seconds of their first agent write. Once any
    // task appears, the function returns `false` and react-query
    // stops polling this key; the paginated `RecentActivity` query
    // takes over. Elsewhere in the app react-query's default
    // no-polling behaviour is unchanged.
    refetchInterval: (query) => {
      const pages = query.state.data?.pages ?? []
      const hasAnyActivity = pages.some((p) => p.items.length > 0)
      return hasAnyActivity ? false : 4000
    },
  })
  // Only count harnesses that appear on the /mcp screen. Hidden ones
  // (Hermes, OpenClaw, Gemini CLI, retired Claude Desktop) may be
  // preinstalled but are never something the reader intentionally
  // connected, so lighting up 'MCP installed' for them is misleading.
  const hasConnection =
    connections.data?.items.some(
      (c) => c.installed && isUserFacingHarness(c.harness),
    ) ?? false
  const connectedHarnesses =
    connections.data?.items
      .filter((c) => c.installed && isUserFacingHarness(c.harness))
      .map((c) => c.harness) ?? []
  const hasHistoricalActivity = (taskProbe.data?.pages ?? []).some(
    (p) => p.items.length > 0,
  )
  const hasLiveSessions = sessions.length > 0

  // Wait for both probes to resolve at least once before deciding
  // which shell to render. Otherwise the onboarding block flashes on
  // first paint for returning users whose tasks are still in-flight.
  const probesResolved =
    connections.data !== undefined && taskProbe.data !== undefined
  const state = hasLiveSessions
    ? 'ready'
    : probesResolved
      ? getOnboardingState({
          hasConnection,
          hasActivity: hasHistoricalActivity,
        })
      : 'ready'
  const stats = useCockpitStats({
    enabled: probesResolved && state === 'ready',
    refetchInterval: 60_000,
  })

  const content =
    state !== 'ready' ? (
      <Page>
        <CockpitOnboarding connectedHarnesses={connectedHarnesses} />
      </Page>
    ) : (
      <Page>
        <PageHeader
          title={`Good ${partOfDay(hour)}`}
          lede={homeLede(sessions, takeover.inControlSession)}
        />
        <TopStrip stats={stats.data} promo={promo} />
        <NeedsYouBanner
          sessions={sessions}
          onTakeOver={takeover.takeOver}
          onStop={takeover.stop}
          pendingTakeOverSessionId={takeover.pendingTakeOverSessionId}
          cancelPendingSessionId={takeover.stopPendingSessionId}
        />
        <RunningGrid
          sessions={sessions}
          onTakeOver={takeover.takeOver}
          takeOverPendingSessionId={takeover.pendingTakeOverSessionId}
        />
        <RecentActivity />
      </Page>
    )

  return (
    <>
      {content}
      {takeover.inControlSession && (
        <InControlBar
          session={takeover.inControlSession}
          onHandBack={takeover.handBack}
          onCancel={takeover.cancelControl}
          isHandingBack={takeover.isHandingBack}
        />
      )}
    </>
  )
}

// ---------------------------------------------------------------------------

/** Time saved and the one promotion, side by side; either takes the row alone. */
function TopStrip({
  stats,
  promo,
}: {
  stats?: CockpitStats
  promo: PromoSlotState
}) {
  const showSaved = (stats?.allTime.sessionCount ?? 0) > 0
  if (!showSaved && promo.kind === null) return null
  return (
    <div
      className={cn(
        'grid grid-cols-1 gap-3.5',
        showSaved &&
          promo.kind !== null &&
          'md:grid-cols-[minmax(0,1fr)_minmax(0,1.45fr)]',
      )}
    >
      {showSaved && stats ? <TimeSavedCard stats={stats} /> : null}
      <PromoSlot {...promo} />
    </div>
  )
}
