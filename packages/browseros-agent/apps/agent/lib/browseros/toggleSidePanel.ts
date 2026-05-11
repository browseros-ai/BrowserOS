import { markRestorePending } from '@/lib/browseros/activeSessionStorage'

/**
 * Session flag: sidepanel chat is waiting on the server / streaming so the
 * background script should keep the side panel visible when CDP activates
 * other tabs.
 */
export const SHIMMY_AGENT_SIDEPANEL_BUSY_KEY = 'shimmyAgentSidepanelBusy' as const

/**
 * Session flag: the tabId that most recently had the side panel open.
 * Used as a fallback on plain Chromium (no browserosIsOpen API) to know
 * whether the panel was open before a tab switch.
 */
export const SHIMMY_SIDEPANEL_LAST_TAB_KEY = 'shimmySidepanelLastTabId' as const

type SidePanelWithBrowseros = typeof chrome.sidePanel & {
  browserosIsOpen?: (opts: { tabId: number }) => Promise<boolean>
  browserosToggle?: (opts: { tabId: number }) => Promise<{ opened: boolean }>
}

/**
 * Record that the side panel is currently open on this tab.
 * Call this whenever the panel is opened or confirmed open.
 * @public
 */
export function recordSidePanelOpenOnTab(tabId: number): void {
  chrome.storage.session
    .set({ [SHIMMY_SIDEPANEL_LAST_TAB_KEY]: tabId })
    .catch(() => null)
}

/**
 * @public
 */
export async function openSidePanel(
  tabId: number,
): Promise<{ opened: boolean }> {
  const sp = chrome.sidePanel as SidePanelWithBrowseros

  if (typeof sp.browserosIsOpen === 'function') {
    try {
      // @ts-expect-error browserosIsOpen is a BrowserOS-specific API
      const isAlreadyOpen = await chrome.sidePanel.browserosIsOpen({ tabId })
      if (isAlreadyOpen) {
        recordSidePanelOpenOnTab(tabId)
        return { opened: true }
      }
    } catch {
      // Non-BrowserOS Chromium — fall through to standard API
    }

    try {
      // @ts-expect-error browserosToggle is a BrowserOS-specific API
      const result = await chrome.sidePanel.browserosToggle({ tabId })
      if (result.opened) recordSidePanelOpenOnTab(tabId)
      return result
    } catch {
      // Fall through
    }
  }

  // Standard Chrome sidePanel API (plain Chromium / dev mode)
  try {
    const tab = await chrome.tabs.get(tabId)
    if (tab.windowId !== undefined) {
      await chrome.sidePanel.setOptions({ tabId, enabled: true })
      await chrome.sidePanel.open({ windowId: tab.windowId })
      recordSidePanelOpenOnTab(tabId)
      return { opened: true }
    }
  } catch {
    // `open` may require a user gesture on some Chrome versions
  }
  return { opened: false }
}

/**
 * @public
 */
export async function toggleSidePanel(
  tabId: number,
): Promise<{ opened: boolean }> {
  const sp = chrome.sidePanel as SidePanelWithBrowseros

  if (typeof sp.browserosToggle === 'function') {
    try {
      // @ts-expect-error browserosToggle is a BrowserOS-specific API
      const result = await chrome.sidePanel.browserosToggle({ tabId })
      if (result.opened) recordSidePanelOpenOnTab(tabId)
      else
        chrome.storage.session
          .remove(SHIMMY_SIDEPANEL_LAST_TAB_KEY)
          .catch(() => null)
      return result
    } catch {
      // Fall through to standard API
    }
  }

  // Standard Chrome sidePanel API fallback
  return openSidePanel(tabId)
}

/**
 * When the active tab changes (e.g. the agent focuses another tab via CDP),
 * re-attach the extension side panel to the new tab if it was open on the
 * previous tab, or if the sidepanel chat is mid-request (`agentBusy`).
 *
 * @public
 */
export async function migrateSidePanelIfOpenBetweenTabs(
  newTabId: number,
  previousTabId: number | undefined,
  agentBusy: boolean,
): Promise<void> {
  if (previousTabId === undefined || previousTabId === newTabId) return

  const sp = chrome.sidePanel as SidePanelWithBrowseros

  if (typeof sp.browserosIsOpen === 'function') {
    try {
      const wasOpen = agentBusy
        ? true
        : await sp.browserosIsOpen({ tabId: previousTabId })
      if (!wasOpen) return
      const already = await sp.browserosIsOpen({ tabId: newTabId })
      if (already) return
      markRestorePending()
      await openSidePanel(newTabId)
    } catch {
      // Non-BrowserOS Chromium or API failure — fall through to standard path
    }
    return
  }

  // Standard Chrome path: check session storage to know if the panel was open
  // on the previous tab (or if the agent is mid-task).
  const sessionData = await chrome.storage.session
    .get(SHIMMY_SIDEPANEL_LAST_TAB_KEY)
    .catch(() => ({} as Record<string, unknown>))
  const lastOpenTabId = (sessionData as Record<string, unknown>)[SHIMMY_SIDEPANEL_LAST_TAB_KEY] as
    | number
    | undefined


  const panelWasOpen =
    agentBusy || lastOpenTabId === previousTabId || lastOpenTabId === newTabId

  if (!panelWasOpen) return

  try {
    const tab = await chrome.tabs.get(newTabId)
    if (tab.windowId !== undefined) {
      await chrome.sidePanel.setOptions({ tabId: newTabId, enabled: true })
      markRestorePending()
      await chrome.sidePanel.open({ windowId: tab.windowId })
      recordSidePanelOpenOnTab(newTabId)
    }
  } catch {
    // `open` may require a user gesture on some Chrome versions
  }
}
