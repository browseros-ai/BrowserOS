import { describe, expect, it } from 'bun:test'
import {
  defaultSearchUrl,
  looksLikeUrl,
  resolveOmniSearch,
} from './newtab-search.helpers'

describe('resolveOmniSearch', () => {
  it('treats a bare host with a dot as a URL and adds https', () => {
    expect(resolveOmniSearch('example.com')).toEqual({
      kind: 'url',
      url: 'https://example.com',
    })
    expect(resolveOmniSearch('foo.bar/baz')).toEqual({
      kind: 'url',
      url: 'https://foo.bar/baz',
    })
  })

  it('keeps an explicit scheme untouched', () => {
    expect(resolveOmniSearch('https://example.com/path')).toEqual({
      kind: 'url',
      url: 'https://example.com/path',
    })
    expect(resolveOmniSearch('http://foo.test')).toEqual({
      kind: 'url',
      url: 'http://foo.test',
    })
  })

  it('recognises localhost with an optional port', () => {
    expect(resolveOmniSearch('localhost:3000')).toEqual({
      kind: 'url',
      url: 'https://localhost:3000',
    })
  })

  it('routes multi-word input and dotless words to a web search', () => {
    expect(resolveOmniSearch('how to make pasta')).toEqual({
      kind: 'search',
      text: 'how to make pasta',
    })
    expect(resolveOmniSearch('weather')).toEqual({
      kind: 'search',
      text: 'weather',
    })
  })

  it('never treats a phrase with spaces as a URL even with a dot', () => {
    expect(looksLikeUrl('example.com and more')).toBe(false)
    expect(resolveOmniSearch('example.com and more')).toEqual({
      kind: 'search',
      text: 'example.com and more',
    })
  })

  it('ignores empty or whitespace-only input', () => {
    expect(resolveOmniSearch('')).toBeNull()
    expect(resolveOmniSearch('   ')).toBeNull()
  })
})

describe('defaultSearchUrl', () => {
  it('encodes the query for the fallback engine', () => {
    expect(defaultSearchUrl('a b & c')).toBe(
      'https://www.google.com/search?q=a%20b%20%26%20c',
    )
  })
})
