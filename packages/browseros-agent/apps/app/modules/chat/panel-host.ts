/** The outer query is native host identity; the hash belongs to screen routing. */
export function panelTabIdFromUrl(url: string): number | undefined {
  const value = new URL(url).searchParams.get('tabId')
  if (value === null || !/^\d+$/.test(value)) return undefined
  const tabId = Number(value)
  return Number.isSafeInteger(tabId) ? tabId : undefined
}
