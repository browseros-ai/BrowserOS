import { beforeEach, describe, expect, it, mock } from 'bun:test'

mock.module('./sidePanelOpenStateStorage', () => ({
  sidePanelPerWindowStorage: { getValue: async () => false },
}))
const { openSidePanel, closeSidePanel, toggleSidePanel, prepareTabSidePanel } =
  await import('./toggleSidePanel')
let opened: Set<number>
let activeTabId = 7
let options: Map<number | undefined, chrome.sidePanel.PanelOptions>
let calls: Array<{ action: string; tabId?: number }>
beforeEach(() => {
  activeTabId = 7
  opened = new Set()
  options = new Map()
  calls = []
  globalThis.chrome = {
    sidePanel: {
      getOptions: async ({ tabId }: { tabId: number }) =>
        options.get(tabId) ?? {},
      setOptions: async (value: chrome.sidePanel.PanelOptions) => {
        options.set(value.tabId, value)
      },
      browserosIsOpen: async ({ tabId }: { tabId: number }) =>
        opened.has(tabId),
      browserosToggle: async ({
        tabId,
        open,
      }: {
        tabId: number
        open?: boolean
      }) => {
        // Custom close works for the active tab and is a no-op for background
        // tabs. Standard close below models its deferred active-tab animation.
        if (open === false) {
          calls.push({ action: 'native-close', tabId })
          if (tabId === activeTabId) opened.delete(tabId)
          return { opened: false }
        }
        expect(open).toBe(true)
        calls.push({ action: 'open', tabId })
        opened.add(tabId)
        return { opened: true }
      },
      close: async ({ tabId }: { tabId: number }) => {
        calls.push({ action: 'close', tabId })
        if (tabId !== activeTabId) opened.delete(tabId)
      },
    },
  } as unknown as typeof chrome
})

describe('tab-specific side panel controls', () => {
  it('registers a user-created tab without opening or inheriting a panel', async () => {
    await prepareTabSidePanel(7)
    expect(options.get(undefined)).toEqual({ enabled: false })
    expect(options.get(7)).toEqual({
      tabId: 7,
      path: 'sidepanel.html?tabId=7',
      enabled: true,
    })
    expect(calls).toEqual([])
  })

  it('closes background B without closing foreground A or disabling registration', async () => {
    await openSidePanel({ tabId: 7, windowId: 3 })
    await openSidePanel({ tabId: 8, windowId: 3 })
    expect(await toggleSidePanel({ tabId: 8, windowId: 3 })).toEqual({
      opened: false,
    })
    expect(opened).toEqual(new Set([7]))
    expect(options.get(8)?.enabled).toBe(true)
    await toggleSidePanel({ tabId: 8, windowId: 3 })
    expect(opened).toEqual(new Set([7, 8]))
  })

  it('keeps the originally captured target and serializes rapid toggles', async () => {
    const target = { tabId: 9, windowId: 4 }
    await Promise.all([toggleSidePanel(target), toggleSidePanel(target)])
    expect(calls).toEqual([
      { action: 'open', tabId: 9 },
      { action: 'native-close', tabId: 9 },
      { action: 'close', tabId: 9 },
    ])
    expect(opened.size).toBe(0)
  })

  it('clears the active tab before a tab switch can interrupt the close animation', async () => {
    opened.add(7)
    await closeSidePanel({ tabId: 7, windowId: 3 })
    activeTabId = 8
    expect(opened.has(7)).toBe(false)
  })

  it('explicit close uses the standard tab API', async () => {
    opened.add(42)
    await closeSidePanel({ tabId: 42, windowId: 4 })
    expect(calls).toEqual([
      { action: 'native-close', tabId: 42 },
      { action: 'close', tabId: 42 },
    ])
  })
})
