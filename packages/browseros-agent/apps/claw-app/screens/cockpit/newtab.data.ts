import type { CockpitStats } from '@browseros/claw-api'
import { type TopSite, useTopSites } from '@/components/newtab/top-sites.hooks'
import { useCockpitStats } from '@/modules/api/cockpit.hooks'
import type { LiveSessionCardRecord } from './cockpit.helpers'
import { pickLeadSession } from './newtab.helpers'

export interface NewtabData {
  sessions: LiveSessionCardRecord[]
  lead: LiveSessionCardRecord | null
  rest: LiveSessionCardRecord[]
  isLive: boolean
  stats: CockpitStats | undefined
  statsPending: boolean
  topSites: TopSite[]
  topSitesPending: boolean
}

/**
 * Single data object for the new-tab monitor. Live sessions are owned by the
 * cockpit screen (they gate the onboarding state machine) and passed in;
 * this hook adds the monitor-specific stats and top-sites and derives the
 * lead / rest split and the live flag.
 */
export function useNewtabData(sessions: LiveSessionCardRecord[]): NewtabData {
  const stats = useCockpitStats()
  const topSites = useTopSites()
  const { lead, rest } = pickLeadSession(sessions)
  return {
    sessions,
    lead,
    rest,
    isLive: sessions.length > 0,
    stats: stats.data,
    statsPending: stats.isPending,
    topSites: topSites.data ?? [],
    topSitesPending: topSites.isPending,
  }
}
