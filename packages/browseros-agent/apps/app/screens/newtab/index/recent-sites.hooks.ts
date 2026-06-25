import { useEffect, useState } from 'react'
import {
  mapTopSitesToRecentSites,
  type RecentSite,
} from './recent-sites.helpers'

const MAX_RECENT_SITES = 5

export function useRecentSites(): RecentSite[] {
  const [recentSites, setRecentSites] = useState<RecentSite[]>([])

  useEffect(() => {
    // `chrome.topSites` is absent until the `topSites` permission is granted
    // (e.g. before the extension reloads with the restored manifest).
    if (!chrome.topSites) return
    chrome.topSites.get().then((sites) => {
      setRecentSites(mapTopSitesToRecentSites(sites, MAX_RECENT_SITES))
    })
  }, [])

  return recentSites
}
