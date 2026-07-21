import { describe, expect, it } from 'bun:test'
import type { CdpConnection } from './connection'
import { PageManager } from './pages'

function tab(targetId: string, url: string) {
  return {
    targetId,
    tabId: 42,
    url,
    title: targetId,
    isActive: true,
    isLoading: false,
    loadProgress: 1,
    isPinned: false,
    isHidden: false,
  }
}

describe('PageManager refresh', () => {
  it('does not overwrite a newer page rebind with a stale response', async () => {
    let tabs = [tab('target-a', 'https://a.example')]
    let releaseRefresh: (value: { tab: ReturnType<typeof tab> }) => void = () =>
      undefined
    let markRefreshStarted: () => void = () => undefined
    const refreshStarted = new Promise<void>((resolve) => {
      markRefreshStarted = resolve
    })
    const staleRefresh = new Promise<{ tab: ReturnType<typeof tab> }>(
      (resolve) => {
        releaseRefresh = resolve
      },
    )
    const cdp = {
      Browser: {
        getTabs: async () => ({ tabs }),
        getTabInfo: async () => {
          markRefreshStarted()
          return staleRefresh
        },
      },
      connectionEpoch: () => 1,
      isConnected: () => true,
    } as unknown as CdpConnection
    const pages = new PageManager(cdp)
    await pages.list()

    const refresh = pages.refresh(1)
    await refreshStarted
    tabs = [tab('target-b', 'https://b.example')]
    await pages.list()
    releaseRefresh({ tab: tab('target-a', 'https://a.example') })

    expect(await refresh).toMatchObject({
      targetId: 'target-b',
      url: 'https://b.example',
    })
    expect(pages.getInfo(1)).toMatchObject({
      targetId: 'target-b',
      url: 'https://b.example',
    })
  })
})
