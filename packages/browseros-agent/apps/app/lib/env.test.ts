import { describe, expect, it } from 'bun:test'
import { parseBrowserOSApiUrl } from './browseros-api-url'
import { parseAlphaFeaturesFlag } from './env'

describe('parseAlphaFeaturesFlag', () => {
  it('defaults alpha features on when unset', () => {
    expect(parseAlphaFeaturesFlag(undefined)).toBe(true)
  })

  it('keeps explicit true enabled', () => {
    expect(parseAlphaFeaturesFlag('true')).toBe(true)
  })

  it('keeps explicit false disabled', () => {
    expect(parseAlphaFeaturesFlag('false')).toBe(false)
  })
})

describe('parseBrowserOSApiUrl', () => {
  it('defaults to the production BrowserOS API when unset', () => {
    expect(parseBrowserOSApiUrl(undefined)).toBe('https://api.browseros.com')
  })

  it('preserves explicit overrides', () => {
    expect(parseBrowserOSApiUrl('http://127.0.0.1:3000')).toBe(
      'http://127.0.0.1:3000',
    )
  })

  it('rejects overrides without a scheme', () => {
    expect(() => parseBrowserOSApiUrl('api.browseros.com')).toThrow(
      'VITE_PUBLIC_BROWSEROS_API must be a valid URL including http:// or https://',
    )
  })

  it('rejects non-HTTP overrides', () => {
    expect(() =>
      parseBrowserOSApiUrl('chrome-extension://extension-id'),
    ).toThrow('VITE_PUBLIC_BROWSEROS_API must use http:// or https://')
  })

  it('returns a URL that can form a valid WXT match pattern', () => {
    expect(`${parseBrowserOSApiUrl(undefined)}/home`).toStartWith('https://')
  })
})
