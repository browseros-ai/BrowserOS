import { describe, expect, it, mock } from 'bun:test'

mock.module('@browseros/shared/constants/limits', () => ({
  CONTENT_LIMITS: {
    CONSOLE_BUFFER_MAX_ENTRIES: 500,
    CONSOLE_DEFAULT_LIMIT: 50,
    CONSOLE_MAX_LIMIT: 200,
    CONSOLE_META_CHAR: 1_000,
  },
}))

mock.module('../../src/lib/logger', () => ({
  logger: {
    debug: mock(() => {}),
    error: mock(() => {}),
    info: mock(() => {}),
    warn: mock(() => {}),
  },
}))

const { Browser } = await import('../../src/browser/browser')

describe('Browser', () => {
  it('creates hidden pages as hidden tabs without opening a new window', async () => {
    const tab = {
      tabId: 10,
      targetId: 'target-10',
      url: 'about:blank',
      title: '',
      isActive: false,
      isLoading: false,
      loadProgress: 1,
      isPinned: false,
      isHidden: true,
      index: 0,
    }
    const cdp = {
      isConnected: () => true,
      onSessionEvent: mock(() => () => {}),
      Target: {
        on: mock(() => {}),
      },
      Browser: {
        createTab: mock(async () => ({ tab })),
        getTabInfo: mock(async () => ({ tab })),
        createWindow: mock(async () => {
          throw new Error('createWindow should not be called for hidden pages')
        }),
      },
    }

    const browser = new Browser(cdp as never)
    const pageId = await browser.newPage('about:blank', {
      hidden: true,
      background: true,
    })

    expect(pageId).toBe(1)
    expect(cdp.Browser.createTab).toHaveBeenCalledWith({
      url: 'about:blank',
      background: true,
      hidden: true,
    })
    expect(cdp.Browser.createWindow).not.toHaveBeenCalled()
  })
})
