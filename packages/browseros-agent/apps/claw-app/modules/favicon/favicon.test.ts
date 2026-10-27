import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { faviconSources, resetFaviconCache } from './favicon'

const globals = globalThis as { chrome?: unknown }
const originalFetch = globalThis.fetch
const DEFAULT_ICON = [1, 2, 3]
const DDG = 'https://icons.duckduckgo.com/ip2/x.com.ico'
const GOOGLE = 'https://www.google.com/s2/favicons?sz=32&domain_url=x.com'
const CACHED =
  'chrome-extension://neo/_favicon/?pageUrl=https%3A%2F%2Fx.com&size=32'

function stubFetch(handler: (url: string) => Promise<Response>) {
  globalThis.fetch = Object.assign(handler, {
    preconnect: originalFetch.preconnect,
  }) as typeof fetch
}

/** The browser's favicon endpoint: known pages get their icon, anything else the default. */
function browserWithIcons(icons: Record<string, number[]>) {
  globals.chrome = {
    runtime: {
      id: 'neo',
      getURL: (path: string) => `chrome-extension://neo${path}`,
    },
  }
  stubFetch(async (input) => {
    const page = new URL(input).searchParams.get('pageUrl') ?? ''
    return new Response(new Uint8Array(icons[page] ?? DEFAULT_ICON))
  })
}

beforeEach(() => resetFaviconCache())

afterEach(() => {
  delete globals.chrome
  globalThis.fetch = originalFetch
})

describe('faviconSources', () => {
  it('starts with the browser cache when it holds the site’s icon', async () => {
    browserWithIcons({ 'https://x.com': [9, 9] })
    expect(await faviconSources('x.com', 32)).toEqual([CACHED, DDG, GOOGLE])
  })

  it('skips the cache when it only has the default icon', async () => {
    browserWithIcons({})
    expect(await faviconSources('https://x.com/home', 32)).toEqual([
      DDG,
      GOOGLE,
    ])
  })

  it('skips the cache when it cannot be read', async () => {
    browserWithIcons({})
    stubFetch(async () => {
      throw new Error('blocked')
    })
    expect(await faviconSources('x.com', 32)).toEqual([DDG, GOOGLE])
  })

  it('uses the web services outside the extension', async () => {
    expect(await faviconSources('x.com', 32)).toEqual([DDG, GOOGLE])
  })

  it('has nothing for an empty site', async () => {
    expect(await faviconSources('', 32)).toEqual([])
  })
})
