import { describe, it } from 'bun:test'
import assert from 'node:assert'
import type {
  CloseTabParams,
  GetTabInfoParams,
  TabInfo,
} from '@browseros/cdp-protocol/domains/browser'
import type { ProtocolApi } from '@browseros/cdp-protocol/protocol-api'
import type { CdpBackend } from '../../src/browser/backends/types'
import { Browser } from '../../src/browser/browser'

class FakeCdpBackend {
  readonly closeTabCalls: CloseTabParams[] = []
  tabs: TabInfo[]

  Browser = {
    getTabs: async () => ({ tabs: this.tabs }),
    getTabInfo: async (params: GetTabInfoParams) => {
      const tab = this.tabs.find(
        (t) => t.tabId === params.tabId || t.targetId === params.targetId,
      )
      if (!tab) throw new Error('Unknown page')
      return { tab }
    },
    closeTab: async (params: CloseTabParams) => {
      this.closeTabCalls.push(params)
      const index = this.tabs.findIndex(
        (t) => t.tabId === params.tabId || t.targetId === params.targetId,
      )
      if (index === -1) throw new Error('Unknown page')
      this.tabs.splice(index, 1)
    },
  }

  Target = {
    on: () => () => {},
  }

  constructor(tabs: TabInfo[]) {
    this.tabs = tabs
  }

  async connect(): Promise<void> {}
  async disconnect(): Promise<void> {}
  isConnected(): boolean {
    return true
  }
  async getTargets(): Promise<[]> {
    return []
  }
  session(): ProtocolApi {
    throw new Error('session not implemented')
  }
  onSessionEvent(): () => void {
    return () => {}
  }
}

function createTab(overrides: Partial<TabInfo>): TabInfo {
  return {
    tabId: 101,
    targetId: 'target-101',
    url: 'https://example.com',
    title: 'Example',
    isActive: false,
    isLoading: false,
    loadProgress: 1,
    isPinned: false,
    isHidden: false,
    windowId: 1,
    index: 0,
    ...overrides,
  }
}

function createBrowser(tabs: TabInfo[]): {
  browser: Browser
  cdp: FakeCdpBackend
} {
  const cdp = new FakeCdpBackend(tabs)
  return {
    browser: new Browser(cdp as unknown as CdpBackend),
    cdp,
  }
}

describe('Browser', () => {
  it('closes listed pages by target ID', async () => {
    const { browser, cdp } = createBrowser([
      createTab({ tabId: 401, targetId: 'target-401' }),
    ])

    const pages = await browser.listPages()
    assert.strictEqual(pages.length, 1)

    await browser.closePage(pages[0].pageId)

    assert.deepStrictEqual(cdp.closeTabCalls, [{ targetId: 'target-401' }])
    assert.strictEqual(cdp.tabs.length, 0)
  })

  it('refreshes pages before treating closePage page IDs as unknown', async () => {
    const { browser, cdp } = createBrowser([
      createTab({ tabId: 402, targetId: 'target-402' }),
    ])

    await browser.closePage(1)

    assert.deepStrictEqual(cdp.closeTabCalls, [{ targetId: 'target-402' }])
    assert.strictEqual(cdp.tabs.length, 0)
  })

  it('retries closePage when the tab target changes before close', async () => {
    const { browser, cdp } = createBrowser([
      createTab({ tabId: 403, targetId: 'target-403-old' }),
    ])

    const pages = await browser.listPages()
    cdp.tabs[0].targetId = 'target-403-new'

    await browser.closePage(pages[0].pageId)

    assert.deepStrictEqual(cdp.closeTabCalls, [
      { targetId: 'target-403-old' },
      { targetId: 'target-403-new' },
    ])
    assert.strictEqual(cdp.tabs.length, 0)
  })
})
