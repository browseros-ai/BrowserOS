import type { CockpitStats } from '@browseros/claw-api'
import { isUserFacingHarness } from '@/components/harness/harness.types'
import type { TopSite } from '@/components/newtab/top-sites.hooks'
import { useTopSites } from '@/components/newtab/top-sites.hooks'
import type { TaskSummary } from '@/modules/api/audit.hooks'
import { useSessions } from '@/modules/api/audit.hooks'
import { useCockpitStats } from '@/modules/api/cockpit.hooks'
import { useConnections } from '@/modules/api/connections.hooks'
import { useCockpitData } from './cockpit.data'
import type { LiveSessionCardRecord } from './cockpit.helpers'
import type { OnboardingState } from './cockpit-onboarding.helpers'
import { getOnboardingState } from './cockpit-onboarding.helpers'

const RECENT_LIMIT = 3

// Structural slices so the dock components stay decoupled from react-query's
// result types and remain trivial to render in tests with plain objects.
export interface StatsSlice {
  data?: CockpitStats
  isPending: boolean
}

export interface RecentSlice {
  data?: { pages: Array<{ items: TaskSummary[] }> }
  isPending: boolean
}

export interface TopSitesSlice {
  data?: TopSite[]
  isPending: boolean
}

export interface NewtabData {
  state: OnboardingState
  isLive: boolean
  sessions: LiveSessionCardRecord[]
  stats: StatsSlice
  recent: RecentSlice
  topSites: TopSitesSlice
}

/**
 * The single aggregation hook for the new-tab ready / live branch. It also owns
 * the onboarding decision so the screen reads one `state` and composes the dock
 * from one object. Stats and top sites only fetch once the reader is past
 * onboarding.
 */
export function useNewtabData(): NewtabData {
  const { sessions } = useCockpitData()
  const connections = useConnections()
  const recent = useSessions({
    variables: { limit: RECENT_LIMIT },
    // Poll every 4s until the first activity lands so the ready handoff is
    // prompt on a fresh profile; react-query stops once any task exists.
    refetchInterval: (query) => {
      const pages = query.state.data?.pages ?? []
      const hasAnyActivity = pages.some((page) => page.items.length > 0)
      return hasAnyActivity ? false : 4000
    },
  })

  const hasConnection =
    connections.data?.items.some(
      (connection) =>
        connection.installed && isUserFacingHarness(connection.harness),
    ) ?? false
  const hasHistoricalActivity = (recent.data?.pages ?? []).some(
    (page) => page.items.length > 0,
  )
  const hasLiveSessions = sessions.length > 0
  const probesResolved =
    connections.data !== undefined && recent.data !== undefined
  const state: OnboardingState = hasLiveSessions
    ? 'ready'
    : probesResolved
      ? getOnboardingState({
          hasConnection,
          hasActivity: hasHistoricalActivity,
        })
      : 'ready'
  const isReady = state === 'ready'

  const stats = useCockpitStats({ enabled: isReady })
  const topSites = useTopSites({ enabled: isReady })

  return {
    state,
    isLive: hasLiveSessions,
    sessions,
    stats,
    recent,
    topSites,
  }
}
