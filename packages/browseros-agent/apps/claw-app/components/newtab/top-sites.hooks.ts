import { createQuery } from 'react-query-kit'

export type TopSite = chrome.topSites.MostVisitedURL

const MAX_TOP_SITES = 8

/**
 * The most-visited shortcuts for the new-tab row. `chrome.topSites.get()` is a
 * local read, not a network call, wrapped in a query for consistent caching
 * and loading state. The API is guarded so a build without the permission
 * returns an empty list instead of throwing.
 */
export const useTopSites = createQuery<TopSite[]>({
  queryKey: ['newtab', 'top-sites'],
  fetcher: async () => {
    const api = globalThis.chrome?.topSites
    if (typeof api?.get !== 'function') return []
    const sites = await api.get()
    return sites.slice(0, MAX_TOP_SITES)
  },
  staleTime: 5 * 60 * 1000,
})
