import {
  openWindowSidePanelIdsStorage,
  sidePanelPerWindowStorage,
} from './sidePanelOpenStateStorage'
import { markRestorePending } from './activeSessionStorage'

export const SHIMMY_AGENT_SIDEPANEL_BUSY_KEY = 'shimmyAgentSidepanelBusy' as const
export const SHIMMY_SIDEPANEL_LAST_TAB_KEY = 'shimmySidepanelLastTabId' as const

export function recordSidePanelOpenOnTab(tabId: number): void {
  chrome.storage.session
    .set({ [SHIMMY_SIDEPANEL_LAST_TAB_KEY]: tabId })
    .catch(() => null)
}

const SIDEPANEL_PATH = 'sidepanel.html'
const openWindowSidePanelIds = new Set<number>()
let sidePanelPerWindow = false
let sidePanelOpenStateListenersRegistered = false
let sidePanelRuntimeStateLoaded = false
let sidePanelRuntimeStateLoadPromise: Promise<void> | null = null
let sidePanelScopePreferenceEpoch = 0
let persistWindowSidePanelOpenStatePromise: Promise<void> = Promise.resolve()

export type SidePanelTarget = {
  tabId: number
  windowId: number
}

export type SidePanelToggleResult = {
  opened: boolean
}

/** Applies an explicit side panel scope change to Chrome and the worker cache. */
export async function setSidePanelPerWindowPreference(
  perWindow: boolean,
): Promise<void> {
  const epoch = sidePanelScopePreferenceEpoch + 1
  sidePanelScopePreferenceEpoch = epoch
  await applySidePanelPerWindowPreference(perWindow, epoch)
}

async function applySidePanelPerWindowPreference(
  perWindow: boolean,
  epoch: number,
): Promise<void> {
  if (epoch !== sidePanelScopePreferenceEpoch) return
  await chrome.sidePanel.setOptions(
    perWindow ? { enabled: true, path: SIDEPANEL_PATH } : { enabled: false },
  )
  cacheSidePanelPerWindowPreference(perWindow, epoch)
}

function cacheSidePanelPerWindowPreference(
  perWindow: boolean,
  epoch: number,
): void {
  if (epoch === sidePanelScopePreferenceEpoch) {
    sidePanelPerWindow = perWindow
  }
}

async function readSidePanelScopePreference(): Promise<boolean> {
  try {
    return await sidePanelPerWindowStorage.getValue()
  } catch {
    return false
  }
}

/** Establishes Chrome's initial side panel options during extension installation. */
export async function initializeSidePanelOptions(): Promise<void> {
  const epoch = sidePanelScopePreferenceEpoch
  const perWindow = await readSidePanelScopePreference()
  await applySidePanelPerWindowPreference(perWindow, epoch)
}

async function loadSidePanelScopePreference(): Promise<void> {
  const epoch = sidePanelScopePreferenceEpoch
  const perWindow = await readSidePanelScopePreference()
  cacheSidePanelPerWindowPreference(perWindow, epoch)
}

async function loadWindowSidePanelOpenState(): Promise<void> {
  const windowIds = await openWindowSidePanelIdsStorage.getValue()
  openWindowSidePanelIds.clear()
  for (const windowId of windowIds) {
    if (Number.isInteger(windowId)) {
      openWindowSidePanelIds.add(windowId)
    }
  }
}

function queuePersistWindowSidePanelOpenState(): void {
  const windowIds = [...openWindowSidePanelIds]
  persistWindowSidePanelOpenStatePromise =
    persistWindowSidePanelOpenStatePromise
      .catch(() => undefined)
      .then(() => openWindowSidePanelIdsStorage.setValue(windowIds))
}

function rememberWindowSidePanelOpen(windowId: number): void {
  if (openWindowSidePanelIds.has(windowId)) return
  openWindowSidePanelIds.add(windowId)
  queuePersistWindowSidePanelOpenState()
}

function rememberWindowSidePanelClosed(windowId: number): void {
  if (!openWindowSidePanelIds.delete(windowId)) return
  queuePersistWindowSidePanelOpenState()
}

/** Refreshes the cached side panel scope and open-window state from storage. */
export async function refreshSidePanelRuntimeState(): Promise<void> {
  await Promise.all([
    loadSidePanelScopePreference(),
    loadWindowSidePanelOpenState(),
  ])
  sidePanelRuntimeStateLoaded = true
}

/** Serializes background startup state before a user-triggered side panel action routes. */
export async function ensureSidePanelRuntimeStateLoaded(): Promise<void> {
  if (sidePanelRuntimeStateLoaded) return
  sidePanelRuntimeStateLoadPromise ??= refreshSidePanelRuntimeState()
    .catch((error) => {
      sidePanelRuntimeStateLoaded = false
      throw error
    })
    .finally(() => {
      sidePanelRuntimeStateLoadPromise = null
    })
  await sidePanelRuntimeStateLoadPromise
}

async function openTabSidePanel({
  tabId,
}: SidePanelTarget): Promise<SidePanelToggleResult> {
  const isAlreadyOpen = await chrome.sidePanel.browserosIsOpen({ tabId })
  if (isAlreadyOpen) {
    return { opened: true }
  }
  return await chrome.sidePanel.browserosToggle({ tabId })
}

async function toggleTabSidePanel({
  tabId,
}: SidePanelTarget): Promise<SidePanelToggleResult> {
  return await chrome.sidePanel.browserosToggle({ tabId })
}

async function openWindowSidePanel({
  windowId,
}: SidePanelTarget): Promise<SidePanelToggleResult> {
  if (!openWindowSidePanelIds.has(windowId)) {
    await chrome.sidePanel.open({ windowId })
    rememberWindowSidePanelOpen(windowId)
  }
  return { opened: true }
}

async function toggleWindowSidePanel(
  target: SidePanelTarget,
): Promise<SidePanelToggleResult> {
  if (openWindowSidePanelIds.has(target.windowId)) {
    await chrome.sidePanel.close({ windowId: target.windowId })
    rememberWindowSidePanelClosed(target.windowId)
    return { opened: false }
  }
  return await openWindowSidePanel(target)
}

/** Tracks standard side panel events so window mode can behave like a toggle. */
export function registerSidePanelOpenStateListeners(): void {
  if (sidePanelOpenStateListenersRegistered) return
  sidePanelOpenStateListenersRegistered = true

  chrome.sidePanel.onOpened.addListener((info) => {
    if (info.tabId === undefined) {
      rememberWindowSidePanelOpen(info.windowId)
    }
  })

  chrome.sidePanel.onClosed.addListener((info) => {
    if (info.tabId === undefined) {
      rememberWindowSidePanelClosed(info.windowId)
    }
  })
}

/** Opens from non-toolbar flows that may not carry Chrome's user gesture. */
export async function openSidePanel(
  target: SidePanelTarget | number,
): Promise<SidePanelToggleResult> {
  await ensureSidePanelRuntimeStateLoaded()
  let normalizedTarget: SidePanelTarget
  if (typeof target === 'number') {
    const tab = await chrome.tabs.get(target).catch(() => null)
    normalizedTarget = {
      tabId: target,
      windowId: tab?.windowId ?? -1,
    }
  } else {
    normalizedTarget = target
  }
  const result = await openTabSidePanel(normalizedTarget)
  if (result.opened) {
    recordSidePanelOpenOnTab(normalizedTarget.tabId)
  }
  return result
}

/** Toggles the configured side panel scope from a toolbar/user gesture. */
export async function toggleSidePanel(
  target: SidePanelTarget | number,
): Promise<SidePanelToggleResult> {
  await ensureSidePanelRuntimeStateLoaded()
  let normalizedTarget: SidePanelTarget
  if (typeof target === 'number') {
    const tab = await chrome.tabs.get(target).catch(() => null)
    normalizedTarget = {
      tabId: target,
      windowId: tab?.windowId ?? -1,
    }
  } else {
    normalizedTarget = target
  }
  let result: SidePanelToggleResult
  if (sidePanelPerWindow) {
    result = await toggleWindowSidePanel(normalizedTarget)
  } else {
    result = await toggleTabSidePanel(normalizedTarget)
  }
  if (result.opened) {
    recordSidePanelOpenOnTab(normalizedTarget.tabId)
  } else {
    chrome.storage.session.remove(SHIMMY_SIDEPANEL_LAST_TAB_KEY).catch(() => null)
  }
  return result
}

/**
 * When the active tab changes (e.g. the agent focuses another tab via CDP),
 * re-attach the extension side panel to the new tab if it was open on the
 * previous tab, or if the sidepanel chat is mid-request (`agentBusy`).
 */
export async function migrateSidePanelIfOpenBetweenTabs(
  newTabId: number,
  previousTabId: number | undefined,
  agentBusy: boolean,
): Promise<void> {
  if (previousTabId === undefined || previousTabId === newTabId) return

  // In window mode, the side panel stays open on the window anyway, so we only migrate in tab mode.
  if (sidePanelPerWindow) return

  try {
    const wasOpen = agentBusy
      ? true
      : await chrome.sidePanel.browserosIsOpen({ tabId: previousTabId })
    if (!wasOpen) return
    const already = await chrome.sidePanel.browserosIsOpen({ tabId: newTabId })
    if (already) return
    markRestorePending()
    // Find the windowId of the new tab
    const tab = await chrome.tabs.get(newTabId)
    if (tab.windowId !== undefined) {
      await openSidePanel({ tabId: newTabId, windowId: tab.windowId })
    }
  } catch {
    // Fail-safe
  }
}
