import { describe, expect, it } from 'bun:test'
import type { ProtocolApi } from '@browseros/cdp-protocol/protocol-api'
import type { PageManager } from './pages'
import { BrowserSession } from './session'

describe('BrowserSession target-bound screenshots', () => {
  it('discards pixels when the page id is rebound during capture', async () => {
    let targetId = 'target-a'
    const pageSession = {
      Page: {
        captureScreenshot: async () => {
          targetId = 'target-b'
          return { data: '/9g=' }
        },
      },
    } as unknown as ProtocolApi
    const pages = {
      getInfo: () => ({ pageId: 7, targetId }),
      getSession: async () => ({
        session: pageSession,
        sessionId: 'cdp-session',
        targetId: 'target-a',
        url: 'https://example.com',
      }),
      refresh: async () => ({ pageId: 7, targetId }),
    } as unknown as PageManager
    const browser = Object.create(BrowserSession.prototype) as BrowserSession
    Object.defineProperties(browser, {
      pages: { value: pages },
      observers: { value: new Map() },
      frames: { value: {} },
    })

    expect(
      await browser.screenshotForTarget(7, 'target-a', {
        format: 'jpeg',
        annotate: false,
      }),
    ).toBeNull()
  })
})
