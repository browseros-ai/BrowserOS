import { beforeAll, beforeEach, describe, expect, it, mock } from 'bun:test'

let storedOpenWindowIds: number[] = []
let storedSidePanelPerWindow: boolean | undefined
let browserosToggleCalls: unknown[] = []
let browserosIsOpenCalls: unknown[] = []
let openCalls: unknown[] = []
let closeCalls: unknown[] = []
let setOptionsCalls: unknown[] = []
let browserosIsOpenResult = false
let getSidePanelPerWindowOverride: (() => Promise<boolean>) | null = null
const onOpenedListeners: Array<
  (info: chrome.sidePanel.PanelOpenedInfo) => void
> = []
const onClosedListeners: Array<
  (info: chrome.sidePanel.PanelClosedInfo) => void
> = []

mock.module('./sidePanelOpenStateStorage', () => ({
  sidePanelPerWindowStorage: {
    getValue: async () => {
      if (getSidePanelPerWindowOverride) {
        return await getSidePanelPerWindowOverride()
      }
      return storedSidePanelPerWindow ?? false
    },
    setValue: async (perWindow: boolean) => {
      storedSidePanelPerWindow = perWindow
    },
  },
  openWindowSidePanelIdsStorage: {
    getValue: async () => storedOpenWindowIds,
    setValue: async (windowIds: number[]) => {
      storedOpenWindowIds = windowIds
    },
  },
}))

let openSidePanel: typeof import('./toggleSidePanel').openSidePanel
let toggleSidePanel: typeof import('./toggleSidePanel').toggleSidePanel
let initializeSidePanelOptions: typeof import('./toggleSidePanel').initializeSidePanelOptions
let registerSidePanelOpenStateListeners: typeof import('./toggleSidePanel').registerSidePanelOpenStateListeners
let refreshSidePanelRuntimeState: typeof import('./toggleSidePanel').refreshSidePanelRuntimeState
let setSidePanelPerWindowPreference: typeof import('./toggleSidePanel').setSidePanelPerWindowPreference

beforeAll(async () => {
  const module = await import('./toggleSidePanel')
  openSidePanel = module.openSidePanel
  toggleSidePanel = module.toggleSidePanel
  initializeSidePanelOptions = module.initializeSidePanelOptions
  registerSidePanelOpenStateListeners =
    module.registerSidePanelOpenStateListeners
  refreshSidePanelRuntimeState = module.refreshSidePanelRuntimeState
  setSidePanelPerWindowPreference = module.setSidePanelPerWindowPreference
})

beforeEach(async () => {
  storedOpenWindowIds = []
  storedSidePanelPerWindow = undefined
  browserosToggleCalls = []
  browserosIsOpenCalls = []
  openCalls = []
  closeCalls = []
  setOptionsCalls = []
  browserosIsOpenResult = false
  getSidePanelPerWindowOverride = null

  globalThis.chrome = {
    sidePanel: {
      browserosToggle: async (options: unknown) => {
        browserosToggleCalls.push(options)
        return { opened: true }
      },
      browserosIsOpen: async (options: unknown) => {
        browserosIsOpenCalls.push(options)
        return browserosIsOpenResult
      },
      open: async (options: unknown) => {
        openCalls.push(options)
      },
      close: async (options: unknown) => {
        closeCalls.push(options)
      },
      setOptions: async (options: unknown) => {
        setOptionsCalls.push(options)
      },
      onOpened: {
        addListener: (
          listener: (info: chrome.sidePanel.PanelOpenedInfo) => void,
        ) => {
          onOpenedListeners.push(listener)
        },
      },
      onClosed: {
        addListener: (
          listener: (info: chrome.sidePanel.PanelClosedInfo) => void,
        ) => {
          onClosedListeners.push(listener)
        },
      },
    },
  } as typeof chrome

  fireWindowClosed(3)
  await setSidePanelPerWindowPreference(false)
  setOptionsCalls = []
})

function fireWindowOpened(windowId: number) {
  for (const listener of onOpenedListeners) {
    listener({ windowId, path: 'sidepanel.html' })
  }
}

function fireWindowClosed(windowId: number) {
  for (const listener of onClosedListeners) {
    listener({ windowId, path: 'sidepanel.html' })
  }
}

describe('side panel scope routing', () => {
  it('hydrates window open state before routing a cold-started toggle', async () => {
    storedSidePanelPerWindow = true
    storedOpenWindowIds = [3]

    const result = await toggleSidePanel({ tabId: 7, windowId: 3 })

    expect(result).toEqual({ opened: false })
    expect(setOptionsCalls).toEqual([])
    expect(closeCalls).toEqual([{ windowId: 3 }])
    expect(openCalls).toEqual([])
  })

  it('keeps toolbar toggles on the BrowserOS tab-specific API when scope storage is absent', async () => {
    const result = await toggleSidePanel({ tabId: 7, windowId: 3 })

    expect(result).toEqual({ opened: true })
    expect(browserosToggleCalls).toEqual([{ tabId: 7 }])
    expect(openCalls).toEqual([])
    expect(closeCalls).toEqual([])
  })

  it('keeps toolbar toggles on the BrowserOS tab-specific API when scope storage is false', async () => {
    storedSidePanelPerWindow = false

    const result = await toggleSidePanel({ tabId: 7, windowId: 3 })

    expect(result).toEqual({ opened: true })
    expect(browserosToggleCalls).toEqual([{ tabId: 7 }])
    expect(openCalls).toEqual([])
    expect(closeCalls).toEqual([])
  })

  it('uses Chromium window APIs when the window-level preference is enabled', async () => {
    registerSidePanelOpenStateListeners()
    await setSidePanelPerWindowPreference(true)

    expect(setOptionsCalls).toEqual([{ enabled: true, path: 'sidepanel.html' }])
    setOptionsCalls = []

    const opened = await toggleSidePanel({ tabId: 7, windowId: 3 })

    expect(opened).toEqual({ opened: true })
    expect(setOptionsCalls).toEqual([])
    expect(openCalls).toEqual([{ windowId: 3 }])
    expect(browserosToggleCalls).toEqual([])

    fireWindowOpened(3)
    const closed = await toggleSidePanel({ tabId: 7, windowId: 3 })

    expect(closed).toEqual({ opened: false })
    expect(closeCalls).toEqual([{ windowId: 3 }])
  })

  it('keeps programmatic opens on the BrowserOS API in window mode', async () => {
    await setSidePanelPerWindowPreference(true)

    const result = await openSidePanel({ tabId: 7, windowId: 3 })

    expect(result).toEqual({ opened: true })
    expect(browserosIsOpenCalls).toEqual([{ tabId: 7 }])
    expect(browserosToggleCalls).toEqual([{ tabId: 7 }])
    expect(openCalls).toEqual([])
    expect(closeCalls).toEqual([])
  })

  it('opens without closing when programmatic opens target an already-open tab panel', async () => {
    await setSidePanelPerWindowPreference(true)
    browserosIsOpenResult = true

    const result = await openSidePanel({ tabId: 7, windowId: 3 })

    expect(result).toEqual({ opened: true })
    expect(browserosIsOpenCalls).toEqual([{ tabId: 7 }])
    expect(browserosToggleCalls).toEqual([])
    expect(openCalls).toEqual([])
    expect(closeCalls).toEqual([])
  })

  it('refreshes the cached scope from extension storage outside the click path', async () => {
    storedSidePanelPerWindow = true

    await refreshSidePanelRuntimeState()
    expect(setOptionsCalls).toEqual([])

    const result = await toggleSidePanel({ tabId: 7, windowId: 3 })

    expect(result).toEqual({ opened: true })
    expect(openCalls).toEqual([{ windowId: 3 }])
    expect(browserosToggleCalls).toEqual([])
  })

  it('falls back to tab scope without changing Chrome options when storage fails', async () => {
    getSidePanelPerWindowOverride = async () => {
      throw new Error('storage unavailable')
    }

    await refreshSidePanelRuntimeState()
    expect(setOptionsCalls).toEqual([])

    const result = await toggleSidePanel({ tabId: 7, windowId: 3 })

    expect(result).toEqual({ opened: true })
    expect(browserosToggleCalls).toEqual([{ tabId: 7 }])
    expect(openCalls).toEqual([])
  })

  it('applies Chrome options for explicit scope changes', async () => {
    await setSidePanelPerWindowPreference(true)
    await setSidePanelPerWindowPreference(false)

    expect(setOptionsCalls).toEqual([
      { enabled: true, path: 'sidepanel.html' },
      { enabled: false },
    ])
  })

  it('initializes Chrome options from the stored scope during installation', async () => {
    storedSidePanelPerWindow = true

    await initializeSidePanelOptions()

    expect(setOptionsCalls).toEqual([{ enabled: true, path: 'sidepanel.html' }])
  })

  it('initializes Chrome options with tab scope when storage fails', async () => {
    getSidePanelPerWindowOverride = async () => {
      throw new Error('storage unavailable')
    }

    await initializeSidePanelOptions()

    expect(setOptionsCalls).toEqual([{ enabled: false }])
  })

  it('keeps a newer explicit setting over stale installation state', async () => {
    let resolveStoredValue: (perWindow: boolean) => void = () => {}
    getSidePanelPerWindowOverride = async () =>
      new Promise<boolean>((resolve) => {
        resolveStoredValue = resolve
      })

    const initializePromise = initializeSidePanelOptions()
    await Promise.resolve()
    await setSidePanelPerWindowPreference(true)
    resolveStoredValue(false)
    await initializePromise

    expect(setOptionsCalls).toEqual([{ enabled: true, path: 'sidepanel.html' }])
  })

  it('keeps a newer explicit setting change over a stale refresh result', async () => {
    let resolveStoredValue: (perWindow: boolean) => void = () => {}
    getSidePanelPerWindowOverride = async () =>
      new Promise<boolean>((resolve) => {
        resolveStoredValue = resolve
      })

    const refreshPromise = refreshSidePanelRuntimeState()
    await Promise.resolve()
    await setSidePanelPerWindowPreference(true)
    resolveStoredValue(false)
    await refreshPromise

    expect(setOptionsCalls).toEqual([{ enabled: true, path: 'sidepanel.html' }])
    const result = await toggleSidePanel({ tabId: 7, windowId: 3 })

    expect(result).toEqual({ opened: true })
    expect(openCalls).toEqual([{ windowId: 3 }])
    expect(browserosToggleCalls).toEqual([])
  })
})
