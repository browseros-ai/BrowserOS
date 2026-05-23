import { useEffect, useMemo, useState } from 'react'

export type ContextType = 'tab' | 'bookmark' | 'memory' | 'file'

export interface ContextItem {
  id: string
  originalId: string
  type: ContextType
  title: string
  subtitle?: string
  url?: string
  icon?: string
}

interface UseContextSourcesOptions {
  enabled: boolean
  filterText?: string
}

interface UseContextSourcesResult {
  items: ContextItem[]
  allItems: ContextItem[]
  isLoading: boolean
}

/**
 * Safkan Unified Context Source Hook
 * Aggregates tabs, bookmarks, and potentially memories/files for the @ picker.
 */
export function useContextSources({
  enabled,
  filterText = '',
}: UseContextSourcesOptions): UseContextSourcesResult {
  const [tabs, setTabs] = useState<ContextItem[]>([])
  const [bookmarks, setBookmarks] = useState<ContextItem[]>([])
  const [isLoading, setIsLoading] = useState(false)

  useEffect(() => {
    if (!enabled) return

    let cancelled = false
    setIsLoading(true)

    // Fetch Tabs
    const fetchTabs = chrome.tabs.query({ currentWindow: true }).then((currentWindowTabs) => {
      if (cancelled) return []
      return currentWindowTabs
        .filter((tab) => tab.url?.startsWith('http'))
        .sort((a, b) => (b.lastAccessed ?? 0) - (a.lastAccessed ?? 0))
        .map((tab) => ({
          id: `tab-${tab.id}`,
          originalId: String(tab.id),
          type: 'tab' as const,
          title: tab.title || 'Untitled Tab',
          subtitle: tab.url,
          url: tab.url,
          icon: tab.favIconUrl,
        }))
    })

    // Fetch Bookmarks
    const fetchBookmarks = new Promise<ContextItem[]>((resolve) => {
      try {
        chrome.bookmarks.search({}, (results) => {
          if (cancelled) return resolve([])
          const mapped = results
            .filter((b) => !!b.url)
            .slice(0, 100)
            .map((b) => ({
              id: `bookmark-${b.id}`,
              originalId: b.id,
              type: 'bookmark' as const,
              title: b.title || 'Untitled Bookmark',
              subtitle: b.url,
              url: b.url,
            }))
          resolve(mapped)
        })
      } catch {
        resolve([])
      }
    })

    Promise.all([fetchTabs, fetchBookmarks])
      .then(([tabsList, bookmarksList]) => {
        if (cancelled) return
        setTabs(tabsList)
        setBookmarks(bookmarksList)
        setIsLoading(false)
      })
      .catch(() => {
        if (cancelled) return
        setIsLoading(false)
      })

    return () => {
      cancelled = true
    }
  }, [enabled])

  const allItems = useMemo(() => [...tabs, ...bookmarks], [tabs, bookmarks])

  const items = useMemo(() => {
    if (!filterText) return allItems
    const search = filterText.toLowerCase()
    
    // Fuzzy match across categories
    const filteredTabs = tabs.filter(
      (t) => t.title.toLowerCase().includes(search) || t.url?.toLowerCase().includes(search)
    )
    const filteredBookmarks = bookmarks.filter(
      (b) => b.title.toLowerCase().includes(search) || b.url?.toLowerCase().includes(search)
    )

    // Return a balanced mix or grouped results
    return [...filteredTabs.slice(0, 10), ...filteredBookmarks.slice(0, 10)]
  }, [allItems, filterText, tabs, bookmarks])

  return { items, allItems, isLoading }
}
