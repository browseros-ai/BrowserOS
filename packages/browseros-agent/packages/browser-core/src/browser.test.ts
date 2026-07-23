import { describe, expect, it } from 'bun:test'
import type { CdpBackend } from './backends/types'
import { Browser } from './browser'

describe('Browser', () => {
  it('creates a normal window when no visible window exists', async () => {
    const createWindowCalls: unknown[] = []
    const createTabCalls: unknown[] = []
    const tab = {
      targetId: 'target-1',
      tabId: 11,
      url: 'https://example.com',
      title: 'Example',
      isActive: false,
      isLoading: false,
      loadProgress: 1,
      isPinned: false,
      windowId: 7,
    }
    const cdp = {
      Browser: {
        getWindows: async () => ({ windows: [] }),
        createWindow: async (params?: unknown) => {
          createWindowCalls.push(params)
          return {
            window: {
              windowId: 7,
              windowType: 'normal',
              bounds: {},
              isActive: true,
              isVisible: true,
              tabCount: 0,
            },
          }
        },
        createTab: async (params: unknown) => {
          createTabCalls.push(params)
          return { tab }
        },
        getTabInfo: async () => ({ tab }),
      },
      Target: {
        on: () => () => {},
      },
      isConnected: () => true,
      connectionEpoch: () => 1,
    } as unknown as CdpBackend
    const browser = new Browser(cdp)

    await expect(
      browser.newPage('https://example.com', { background: true }),
    ).resolves.toBe(1)

    expect(createWindowCalls).toEqual([undefined])
    expect(createTabCalls).toEqual([
      {
        url: 'https://example.com',
        background: true,
        windowId: 7,
      },
    ])
  })
})
