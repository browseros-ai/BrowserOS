import { afterEach, describe, expect, it } from 'bun:test'
import {
  browserFaviconUrl,
  duckDuckGoFaviconUrl,
  googleFaviconUrl,
  siteHost,
} from './favicon.helpers'

const globals = globalThis as { chrome?: unknown }

afterEach(() => {
  delete globals.chrome
})

describe('siteHost', () => {
  it('reads the host from a bare site or a full address', () => {
    expect(siteHost('news.ycombinator.com')).toBe('news.ycombinator.com')
    expect(siteHost('https://X.com/home?tab=1')).toBe('x.com')
    expect(siteHost('http://localhost:3000/app')).toBe('localhost')
  })

  it('has nothing for an empty site', () => {
    expect(siteHost('  ')).toBeNull()
  })
})

describe('icon addresses', () => {
  it('asks the browser cache for a page inside the extension only', () => {
    expect(browserFaviconUrl('https://x.com', 32)).toBeNull()
    globals.chrome = {
      runtime: {
        id: 'neo',
        getURL: (path: string) => `chrome-extension://neo${path}`,
      },
    }
    expect(browserFaviconUrl('https://x.com', 32)).toBe(
      'chrome-extension://neo/_favicon/?pageUrl=https%3A%2F%2Fx.com&size=32',
    )
  })

  it('builds the DuckDuckGo and Google fallbacks from the host', () => {
    expect(duckDuckGoFaviconUrl('stackoverflow.com')).toBe(
      'https://icons.duckduckgo.com/ip2/stackoverflow.com.ico',
    )
    expect(googleFaviconUrl('stackoverflow.com', 32)).toBe(
      'https://www.google.com/s2/favicons?sz=32&domain_url=stackoverflow.com',
    )
  })
})
