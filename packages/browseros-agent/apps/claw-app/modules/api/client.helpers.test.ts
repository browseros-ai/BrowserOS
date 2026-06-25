import { describe, expect, it } from 'bun:test'
import { resolveApiBaseUrlFromSources } from './client.helpers'

const fallback = 'http://127.0.0.1:9200/cockpit'

describe('resolveApiBaseUrlFromSources', () => {
  it('prefers the query override', () => {
    expect(
      resolveApiBaseUrlFromSources({
        query: 'http://127.0.0.1:9200/cockpit',
        stored: 'http://127.0.0.1:9300/cockpit',
        launcher: 'http://127.0.0.1:9400/cockpit',
        fallback,
      }),
    ).toBe('http://127.0.0.1:9200/cockpit')
  })

  it('uses session storage before the launcher env', () => {
    expect(
      resolveApiBaseUrlFromSources({
        query: null,
        stored: 'http://127.0.0.1:9300/cockpit',
        launcher: 'http://127.0.0.1:9400/cockpit',
        fallback,
      }),
    ).toBe('http://127.0.0.1:9300/cockpit')
  })

  it('uses the launcher env before the default fallback', () => {
    expect(
      resolveApiBaseUrlFromSources({
        query: null,
        stored: null,
        launcher: 'http://127.0.0.1:9400/cockpit',
        fallback,
      }),
    ).toBe('http://127.0.0.1:9400/cockpit')
  })

  it('ignores non-loopback overrides', () => {
    expect(
      resolveApiBaseUrlFromSources({
        query: 'https://example.com/cockpit',
        stored: 'http://localhost:9300/cockpit',
        launcher: 'http://0.0.0.0:9400/cockpit',
        fallback,
      }),
    ).toBe(fallback)
  })

  it('rejects loopback-looking URLs that parse to another host', () => {
    expect(
      resolveApiBaseUrlFromSources({
        query: 'http://127.0.0.1:@example.com/cockpit',
        stored: null,
        launcher: null,
        fallback,
      }),
    ).toBe(fallback)
  })

  it('rejects malformed ports and non-cockpit paths', () => {
    expect(
      resolveApiBaseUrlFromSources({
        query: 'http://127.0.0.1:99999/cockpit',
        stored: 'http://127.0.0.1:9300',
        launcher: 'http://127.0.0.1:9400/cockpit?x=1',
        fallback,
      }),
    ).toBe(fallback)
  })
})
