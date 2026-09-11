import { sidePanelPerWindowStorage } from './sidePanelOpenStateStorage'

export type SidePanelTarget = { tabId: number; windowId: number }
export type SidePanelToggleResult = { opened: boolean }

let initialization: Promise<void> | undefined
const operations = new Map<number, Promise<unknown>>()

/**
 * Panel visibility belongs to each tab. Disable the native global entry on every
 * worker startup (Chromium forgets options on extension reload). Storage version
 * migration separately resets the retired sharing preference once per profile.
 */
export function initializeSidePanelOptions(): Promise<void> {
  initialization ??= (async () => {
    await sidePanelPerWindowStorage.getValue()
    await chrome.sidePanel.setOptions({ enabled: false })
  })().catch((error) => {
    initialization = undefined
    throw error
  })
  return initialization
}

/** Register ownership without opening. This also gives native Alt+A a fixed URL. */
export async function prepareTabSidePanel(tabId: number): Promise<void> {
  await initializeSidePanelOptions()
  const path = `sidepanel.html?tabId=${tabId}`
  const options = await chrome.sidePanel.getOptions({ tabId })
  if (options.path !== path || !options.enabled) {
    await chrome.sidePanel.setOptions({ tabId, path, enabled: true })
  }
}

/** Opens a captured target; never queries whichever tab became active meanwhile. */
export function openSidePanel(
  target: SidePanelTarget,
): Promise<SidePanelToggleResult> {
  return forTab(target.tabId, async () => {
    await prepareTabSidePanel(target.tabId)
    return chrome.sidePanel.browserosToggle({ tabId: target.tabId, open: true })
  })
}

/**
 * The shipped APIs have complementary close behavior: custom close clears the
 * active entry immediately (before animation), while standard close clears a
 * background entry. Use both, without disabling the panel or guessing whether
 * its tab is still active after asynchronous calls.
 */
export function closeSidePanel(
  target: SidePanelTarget,
): Promise<SidePanelToggleResult> {
  return forTab(target.tabId, async () => {
    return closeTabPanel(target.tabId)
  })
}

/** Until the native toggle is fixed, use its correct state reader + explicit action. */
export function toggleSidePanel(
  target: SidePanelTarget,
): Promise<SidePanelToggleResult> {
  return forTab(target.tabId, async () => {
    await prepareTabSidePanel(target.tabId)
    if (await chrome.sidePanel.browserosIsOpen({ tabId: target.tabId })) {
      return closeTabPanel(target.tabId)
    }
    return chrome.sidePanel.browserosToggle({ tabId: target.tabId, open: true })
  })
}

async function closeTabPanel(tabId: number): Promise<SidePanelToggleResult> {
  await chrome.sidePanel.browserosToggle({ tabId, open: false })
  await chrome.sidePanel.close({ tabId })
  return { opened: false }
}

// Serialize extension actions per tab across asynchronous API calls. Native
// Alt+A executes outside this queue; its atomic implementation remains Chromium's.
function forTab<T>(tabId: number, action: () => Promise<T>): Promise<T> {
  const next = (operations.get(tabId) ?? Promise.resolve())
    .catch(() => undefined)
    .then(action)
  operations.set(tabId, next)
  void next
    .finally(() => {
      if (operations.get(tabId) === next) operations.delete(tabId)
    })
    .catch(() => undefined)
  return next
}
