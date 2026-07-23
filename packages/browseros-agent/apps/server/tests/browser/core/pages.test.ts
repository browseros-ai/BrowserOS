import { describe, expect, it } from 'bun:test'
import type { CdpConnection } from '@browseros/browser-core/core/connection'
import { PageManager } from '@browseros/browser-core/core/pages'

type FakeTab = {
  targetId: string
  tabId: number
  url: string
  title: string
  isActive: boolean
  isLoading: boolean
  loadProgress: number
  isPinned: boolean
  windowId: number
}

function createPageManagerHarness() {
  const tabs = new Map<number, FakeTab>()
  const createTabCalls: Array<Record<string, unknown>> = []
  let nextTabId = 1

  const cdp = {
    isConnected: () => true,
    connectionEpoch: () => 1,
    session: () => ({}),
    Browser: {
      createTab: async (params: Record<string, unknown>) => {
        createTabCalls.push(params)
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
    createTabCalls,
  }
}

describe('PageManager', () => {
  it('preserves background and explicit-window creation', async () => {
    const { manager, createTabCalls } = createPageManagerHarness()

    const pageId = await manager.newPage('https://example.com', {
      background: true,
      windowId: 100,
    })

    expect(createTabCalls).toEqual([
      {
        url: 'https://example.com',
        background: true,
        windowId: 100,
      },
    ])
    expect(manager.getInfo(pageId)?.windowId).toBe(100)
  })
})
