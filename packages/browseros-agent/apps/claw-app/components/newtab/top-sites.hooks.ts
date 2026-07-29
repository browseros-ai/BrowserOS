import { createQuery } from 'react-query-kit'

export interface TopSite {
  url: string
  title: string
}

const TOP_SITES_LIMIT = 8

/**
 * Most-visited shortcuts from the browser's own history. Wrapped in a query
 * for caching, loading, and empty handling even though the source is a local
 * promise rather than a network call.
 */
export const useTopSites = createQuery<TopSite[]>({
  queryKey: ['chrome', 'topSites'],
  fetcher: async () => {
    if (typeof chrome === 'undefined' || !chrome.topSites?.get) return []
    const items = await chrome.topSites.get()
    return items.slice(0, TOP_SITES_LIMIT).map((item) => ({
      url: item.url,
      title: item.title,
    }))
  },
  staleTime: 5 * 60 * 1000,
})

/**
 * Extension `_favicon` URL for a page. Icons resolve from the local cache so
 * nothing leaves the machine; returns null where the API is unavailable so
 * the caller can fall back to a neutral glyph.
 */
export function faviconUrl(pageUrl: string): string | null {
  if (typeof chrome === 'undefined' || !chrome.runtime?.getURL) return null
  return chrome.runtime.getURL(
    `/_favicon/?pageUrl=${encodeURIComponent(pageUrl)}&size=32`,
  )
}
