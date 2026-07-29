import { type TopSite, useTopSites } from '@/components/newtab/top-sites.hooks'
import { type TaskSummary, useSessions } from '@/modules/api/audit.hooks'

const RECENT_LIMIT = 3

export interface NewtabData {
  topSites: TopSite[]
  topSitesPending: boolean
  recent: TaskSummary[]
  recentPending: boolean
}

/** Aggregates the browse-surface data the new-tab needs beyond live sessions. */
export function useNewtabData(): NewtabData {
  const topSites = useTopSites()
  const recent = useSessions({
    variables: { limit: RECENT_LIMIT },
    refetchInterval: 3000,
  })
  const recentTasks = (recent.data?.pages ?? [])
    .flatMap((page) => page.items)
    .slice(0, RECENT_LIMIT)

  return {
    topSites: topSites.data ?? [],
    topSitesPending: topSites.isPending,
    recent: recentTasks,
    recentPending: recent.isPending,
  }
}
