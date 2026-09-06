/** The outer query is native host identity; the hash belongs to screen routing. */
export function panelTabIdFromUrl(url: string): number | undefined {
  const value = new URL(url).searchParams.get('tabId')
  if (value === null || !/^\d+$/.test(value)) return undefined
  const tabId = Number(value)
  return Number.isSafeInteger(tabId) ? tabId : undefined
}

/** Window-panel context is readable, but never authorizes taking over that tab. */
export function getSubmittingPanelTabId(
  origin: 'sidepanel' | 'newtab' | undefined,
  contextualHost: number | undefined,
  contextTab: number | undefined,
): number | undefined {
  if (contextualHost !== undefined) return contextualHost
  return origin === 'newtab' ? contextTab : undefined
}
