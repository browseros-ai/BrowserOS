import { afterEach, describe, expect, it } from 'bun:test'
import { faviconUrl } from './site-badge.helpers'

const globals = globalThis as { chrome?: unknown }
const EXTENSION = 'chrome-extension://neo-test-id'

function inExtension() {
  globals.chrome = {
    runtime: {
      id: 'neo-test-id',
      getURL: (path: string) => `${EXTENSION}${path}`,
    },
  }
}

afterEach(() => {
  delete globals.chrome
})

describe('faviconUrl', () => {
  it('asks the browser for the icon of the site’s home page', () => {
    inExtension()
    const url = new URL(faviconUrl('news.ycombinator.com') ?? '')
    expect(url.href.split('?')[0]).toBe(`${EXTENSION}/_favicon/`)
    expect(url.searchParams.get('pageUrl')).toBe('https://news.ycombinator.com')
    expect(url.searchParams.get('size')).toBe('32')
  })

  it('keeps a site that is already a full address', () => {
    inExtension()
    const url = new URL(faviconUrl('http://localhost:3000', 64) ?? '')
    expect(url.searchParams.get('pageUrl')).toBe('http://localhost:3000')
    expect(url.searchParams.get('size')).toBe('64')
  })

  it('has nothing to offer outside the extension', () => {
    expect(faviconUrl('x.com')).toBeNull()
  })
})
