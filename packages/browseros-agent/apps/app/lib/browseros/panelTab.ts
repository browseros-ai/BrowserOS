/** Native contextual panel URLs carry ownership; tab activation never changes it. */
export function panelTabIdFromUrl(url: string): number | undefined {
  const value = new URL(url).searchParams.get('tabId')
  if (value === null || !/^\d+$/.test(value)) return undefined
  const tabId = Number(value)
  return Number.isSafeInteger(tabId) ? tabId : undefined
}

export async function resolvePanelTabId(): Promise<number | undefined> {
  const tabId = panelTabIdFromUrl(location.href)
  if (tabId !== undefined) return tabId
  // Compatibility for an already-registered native panel from before the
  // extension update. Capture once on mount, never subscribe to tab activation.
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true })
  return tab?.id
}
