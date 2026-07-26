import { useEffect, useMemo, useState } from 'react'

export interface UseAvailableTabsOptions {
  enabled: boolean
  filterText?: string
}

export interface UseAvailableTabsResult {
  tabs: chrome.tabs.Tab[]
  allTabs: chrome.tabs.Tab[]
  isLoading: boolean
}

export function useAvailableTabs({
  enabled,
  filterText = '',
}: UseAvailableTabsOptions): UseAvailableTabsResult {
  const [allTabs, setAllTabs] = useState<chrome.tabs.Tab[]>([])
  const [isLoading, setIsLoading] = useState(false)

  useEffect(() => {
    if (!enabled) return

    let cancelled = false
    setIsLoading(true)

    chrome.tabs
      .query({ currentWindow: true })
      .then((currentWindowTabs) => {
        if (cancelled) return
        const httpTabs = currentWindowTabs
          .filter((tab) => tab.url?.startsWith('http'))
          .sort((a, b) => (b.lastAccessed ?? 0) - (a.lastAccessed ?? 0))
        setAllTabs(httpTabs)
        setIsLoading(false)
      })
      .catch((_error) => {
        if (cancelled) return
        setAllTabs([])
        setIsLoading(false)
      })

    return () => {
      cancelled = true
    }
  }, [enabled])

  const tabs = useMemo(() => {
    if (!filterText) return allTabs
    const search = filterText.toLowerCase()
    return allTabs.filter(
      (tab) =>
        tab.title?.toLowerCase().includes(search) ||
        tab.url?.toLowerCase().includes(search),
    )
  }, [allTabs, filterText])

  return { tabs, allTabs, isLoading }
}
