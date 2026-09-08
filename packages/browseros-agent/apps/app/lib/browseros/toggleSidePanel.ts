import {
  openWindowSidePanelIdsStorage,
  sidePanelPerWindowStorage,
} from './sidePanelOpenStateStorage'

const SIDEPANEL_PATH = 'sidepanel.html'
// All writers in the worker share this queue, including toolbar and broker.
const targetOperations = new Map<string, Promise<SidePanelToggleResult>>()
let scopeOptionsWrite: Promise<void> = Promise.resolve()
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
  const write = scopeOptionsWrite
    .catch(() => undefined)
    .then(async () => {
      if (epoch !== sidePanelScopePreferenceEpoch) return
      await chrome.sidePanel.setOptions(
        perWindow
          ? { enabled: true, path: SIDEPANEL_PATH }
          : { enabled: false },
      )
      cacheSidePanelPerWindowPreference(perWindow, epoch)
    })
  scopeOptionsWrite = write
  await write
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
  // Updates and worker restarts must also disable the global default in tab
  // mode. A leftover global panel otherwise leaks into newly opened tabs.
  await applySidePanelPerWindowPreference(perWindow, epoch)
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
  // Encode native host identity, never conversation/run identity: changing a
  // path destroys Chrome's cached panel view and its in-progress draft.
  await chrome.sidePanel.setOptions({
    tabId,
    enabled: true,
    path: `${SIDEPANEL_PATH}?tabId=${tabId}`,
  })
  return await chrome.sidePanel.browserosToggle({ tabId, open: true })
}

async function toggleTabSidePanel(
  target: SidePanelTarget,
): Promise<SidePanelToggleResult> {
  // Native implicit toggle only checks foreground visibility. IsOpen includes
  // inactive tabs' remembered contextual state; standard close clears it safely.
  if (await chrome.sidePanel.browserosIsOpen({ tabId: target.tabId })) {
    await chrome.sidePanel.close({ tabId: target.tabId })
    return { opened: false }
  }
  return await openTabSidePanel(target)
}

function forTarget(
  key: string,
  action: () => Promise<SidePanelToggleResult>,
): Promise<SidePanelToggleResult> {
  const next = (targetOperations.get(key) ?? Promise.resolve())
    .catch(() => undefined)
    .then(action)
  targetOperations.set(key, next)
  void next
    .finally(() => {
      if (targetOperations.get(key) === next) targetOperations.delete(key)
    })
    .catch(() => undefined)
  return next
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
  target: SidePanelTarget,
): Promise<SidePanelToggleResult> {
  await ensureSidePanelRuntimeStateLoaded()
  return await forTarget(`tab:${target.tabId}`, () => openTabSidePanel(target))
}

/** Toggles the configured side panel scope from a toolbar/user gesture. */
export async function toggleSidePanel(
  target: SidePanelTarget,
): Promise<SidePanelToggleResult> {
  await ensureSidePanelRuntimeStateLoaded()
  if (sidePanelPerWindow) {
    return await forTarget(`window:${target.windowId}`, () =>
      toggleWindowSidePanel(target),
    )
  }
  return await forTarget(`tab:${target.tabId}`, () =>
    toggleTabSidePanel(target),
  )
}
