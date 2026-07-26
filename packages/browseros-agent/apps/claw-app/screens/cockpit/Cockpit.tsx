import { CockpitHero } from '@/components/cockpit/CockpitHero'
import { CockpitOnboarding } from '@/components/cockpit/CockpitOnboarding'
import { RecentActivity } from '@/components/cockpit/RecentActivity'
import { RunningGrid } from '@/components/cockpit/RunningGrid'
import { SavedStatsBand } from '@/components/cockpit/SavedStatsBand'
import { isUserFacingHarness } from '@/components/harness/harness.types'
import { useSessions } from '@/modules/api/audit.hooks'
import { useCockpitStats } from '@/modules/api/cockpit.hooks'
import { useConnections } from '@/modules/api/connections.hooks'
import { useCockpitData } from './cockpit.data'
import { getOnboardingState } from './cockpit-onboarding.helpers'

const ONBOARDING_PROBE_LIMIT = 1

/** Renders the Claw cockpit homepage. */
export function Cockpit() {
  const { sessions } = useCockpitData()

  // When no live session is connected, these probes decide which onboarding
  // shell to show. Their stable keys are shared with RecentActivity and MCP.
  const connections = useConnections()
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
  const shouldLoadStats =
    probesResolved && state === 'ready' && !hasLiveSessions
  const stats = useCockpitStats({
    enabled: shouldLoadStats,
  })

  if (state !== 'ready') {
    return (
      <div className="mx-auto flex max-w-7xl flex-col px-8 pt-8 pb-16">
        <CockpitOnboarding state={state} />
      </div>
    )
  }

  return (
    <div className="mx-auto flex max-w-7xl flex-col gap-8 px-8 pt-8 pb-16">
      <CockpitHero />
      {shouldLoadStats && stats.data?.hasMeasuredStats ? (
        <SavedStatsBand stats={stats.data} />
      ) : (
        <RunningGrid sessions={sessions} />
      )}
      <RecentActivity />
    </div>
  )
}
