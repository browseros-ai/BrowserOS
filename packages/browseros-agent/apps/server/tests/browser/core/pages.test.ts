import { describe, expect, it } from 'bun:test'
import type { CdpConnection } from '@browseros/browser-core/core/connection'
import { PageManager } from '@browseros/browser-core/core/pages'

type FakeWindow = {
  windowId: number
  isVisible: boolean
  isActive: boolean
}

type FakeTab = {
  targetId: string
  tabId: number
  url: string
  title: string
  isActive: boolean
  isLoading: boolean
  loadProgress: number
  isPinned: boolean
  isHidden: boolean
  windowId: number
}

function createPageManagerHarness() {
  const windows: FakeWindow[] = []
  const tabs = new Map<number, FakeTab>()
  const createWindowCalls: Array<{ hidden?: boolean }> = []
  const createTabCalls: Array<Record<string, unknown>> = []
  let nextWindowId = 100
  let nextTabId = 1

  const cdp = {
    isConnected: () => true,
    connectionEpoch: () => 1,
    session: () => ({}),
    Browser: {
      getWindows: async () => ({ windows }),
      createWindow: async (opts: { hidden?: boolean }) => {
        createWindowCalls.push(opts)
        const window = {
          windowId: nextWindowId++,
          isVisible: !opts.hidden,
          isActive: false,
        }
        windows.push(window)
        return { window }
      },
      createTab: async (params: Record<string, unknown>) => {
        createTabCalls.push(params)
        const window = windows.find(
          (candidate) => candidate.windowId === params.windowId,
        )
        const tabId = nextTabId++
        const tab = {
          targetId: `target-${tabId}`,
          tabId,
          url: String(params.url),
          title: '',
          isActive: false,
          isLoading: false,
          loadProgress: 1,
          isPinned: false,
          isHidden: window ? !window.isVisible : false,
          windowId: Number(params.windowId),
        }
        tabs.set(tabId, tab)
        return { tab }
      },
      getTabInfo: async ({ tabId }: { tabId: number }) => ({
        tab: tabs.get(tabId),
      }),
    },
  } as unknown as CdpConnection

  return {
    manager: new PageManager(cdp),
    createWindowCalls,
    createTabCalls,
  }
}

describe('PageManager', () => {
  it('reuses the hidden window it creates for hidden pages', async () => {
    const { manager, createWindowCalls, createTabCalls } =
      createPageManagerHarness()

    await manager.newPage('https://first.example', {
      hidden: true,
      background: true,
    })
    await manager.newPage('https://second.example', {
      hidden: true,
      background: true,
    })

    expect(createWindowCalls).toEqual([{ hidden: true }])
    expect(createTabCalls).toEqual([
      expect.objectContaining({ windowId: 100 }),
      expect.objectContaining({ windowId: 100 }),
    ])
  })
})
