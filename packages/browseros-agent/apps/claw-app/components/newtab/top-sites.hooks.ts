import { createQuery } from 'react-query-kit'

export interface TopSite {
  url: string
  title: string
}

const TOP_SITES_LIMIT = 8

/** Most-visited shortcuts for the new-tab row, capped and read on the device. */
export const useTopSites = createQuery<TopSite[]>({
  queryKey: ['chrome', 'topSites'],
  fetcher: async () => {
    if (
      typeof chrome === 'undefined' ||
      typeof chrome.topSites?.get !== 'function'
    ) {
      return []
    }
    const sites = await chrome.topSites.get()
    return sites
      .slice(0, TOP_SITES_LIMIT)
      .map((site) => ({ url: site.url, title: site.title }))
  },
})

/**
 * Extension-local favicon endpoint so a site's icon renders without a request
 * to any third party. Null when the runtime API is unavailable (tests, non
 * extension contexts) so callers fall back to a neutral glyph.
 */
export function faviconUrl(pageUrl: string): string | null {
  if (
    typeof chrome === 'undefined' ||
    typeof chrome.runtime?.getURL !== 'function'
  ) {
    return null
  }
  const endpoint = new URL(chrome.runtime.getURL('/_favicon/'))
  endpoint.searchParams.set('pageUrl', pageUrl)
  endpoint.searchParams.set('size', '32')
  return endpoint.toString()
}
