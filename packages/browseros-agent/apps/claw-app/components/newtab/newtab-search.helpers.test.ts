import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { looksLikeUrl, submitOmni, toUrl } from './newtab-search.helpers'

describe('looksLikeUrl', () => {
  it('treats explicit schemes and dotted hosts as URLs', () => {
    expect(looksLikeUrl('https://example.com')).toBe(true)
    expect(looksLikeUrl('chrome://settings')).toBe(true)
    expect(looksLikeUrl('example.com')).toBe(true)
    expect(looksLikeUrl('docs.browseros.com/guide')).toBe(true)
  })

  it('treats plain words and multi-word phrases as queries', () => {
    expect(looksLikeUrl('weather')).toBe(false)
    expect(looksLikeUrl('best pasta recipe')).toBe(false)
    expect(looksLikeUrl('claw vs codex')).toBe(false)
    expect(looksLikeUrl('  ')).toBe(false)
  })
})

describe('toUrl', () => {
  it('keeps an existing scheme and adds https when scheme-less', () => {
    expect(toUrl('https://example.com')).toBe('https://example.com')
    expect(toUrl('example.com')).toBe('https://example.com')
    expect(toUrl('chrome://settings')).toBe('chrome://settings')
  })
})

describe('submitOmni', () => {
  let assigned: string[]
  let searchQueries: Array<{ text: string; disposition: string }>
  const originalWindow = Reflect.getOwnPropertyDescriptor(globalThis, 'window')
  const originalChrome = Reflect.getOwnPropertyDescriptor(globalThis, 'chrome')

  beforeEach(() => {
    assigned = []
    searchQueries = []
    Object.defineProperty(globalThis, 'window', {
      configurable: true,
      writable: true,
      value: { location: { assign: (url: string) => assigned.push(url) } },
    })
  })

  afterEach(() => {
    if (originalWindow)
      Object.defineProperty(globalThis, 'window', originalWindow)
    else Reflect.deleteProperty(globalThis, 'window')
    if (originalChrome)
      Object.defineProperty(globalThis, 'chrome', originalChrome)
    else Reflect.deleteProperty(globalThis, 'chrome')
  })

  it('navigates the current tab when the input is a URL', () => {
    Reflect.deleteProperty(globalThis, 'chrome')
    submitOmni('example.com')
    expect(assigned).toEqual(['https://example.com'])
    expect(searchQueries).toEqual([])
  })

  it('runs a default-engine search for a query when chrome.search is present', () => {
    Object.defineProperty(globalThis, 'chrome', {
      configurable: true,
      writable: true,
      value: {
        search: {
          query: (info: { text: string; disposition: string }) =>
            searchQueries.push(info),
        },
      },
    })
    submitOmni('best pasta recipe')
    expect(searchQueries).toEqual([
      { text: 'best pasta recipe', disposition: 'CURRENT_TAB' },
    ])
    expect(assigned).toEqual([])
  })

  it('falls back to a default search URL when chrome.search is unavailable', () => {
    Reflect.deleteProperty(globalThis, 'chrome')
    submitOmni('hello world')
    expect(assigned).toEqual(['https://www.google.com/search?q=hello%20world'])
  })

  it('ignores an empty submission', () => {
    Reflect.deleteProperty(globalThis, 'chrome')
    submitOmni('   ')
    expect(assigned).toEqual([])
    expect(searchQueries).toEqual([])
  })
})
