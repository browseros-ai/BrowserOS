import { describe, expect, it } from 'bun:test'
import {
  looksLikeUrl,
  normalizeUrl,
  resolveOmni,
} from './newtab-search.helpers'

describe('looksLikeUrl', () => {
  it('treats schemes and dotted hosts as URLs', () => {
    expect(looksLikeUrl('https://example.com')).toBe(true)
    expect(looksLikeUrl('example.com')).toBe(true)
    expect(looksLikeUrl('news.kagi.com/path')).toBe(true)
    expect(looksLikeUrl('localhost:3000')).toBe(true)
  })

  it('treats prose and dotless or malformed tokens as searches', () => {
    expect(looksLikeUrl('how to book a flight')).toBe(false)
    expect(looksLikeUrl('claude')).toBe(false)
    expect(looksLikeUrl('example dot com')).toBe(false)
    expect(looksLikeUrl('.hidden')).toBe(false)
    expect(looksLikeUrl('trailing.')).toBe(false)
    expect(looksLikeUrl('')).toBe(false)
  })
})

describe('normalizeUrl', () => {
  it('prepends https when no scheme is present', () => {
    expect(normalizeUrl('example.com')).toBe('https://example.com')
  })

  it('keeps an existing scheme', () => {
    expect(normalizeUrl('http://a.test/x')).toBe('http://a.test/x')
  })
})

describe('resolveOmni', () => {
  it('routes a URL to navigate with a normalized scheme', () => {
    expect(resolveOmni('example.com')).toEqual({
      kind: 'navigate',
      url: 'https://example.com',
    })
    expect(resolveOmni('https://a.test/x')).toEqual({
      kind: 'navigate',
      url: 'https://a.test/x',
    })
  })

  it('routes prose to a web search', () => {
    expect(resolveOmni('best coffee near me')).toEqual({
      kind: 'search',
      text: 'best coffee near me',
    })
  })

  it('ignores empty input', () => {
    expect(resolveOmni('   ')).toBeNull()
  })
})
