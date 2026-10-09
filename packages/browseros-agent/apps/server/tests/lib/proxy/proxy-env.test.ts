import { describe, expect, it } from 'bun:test'
import {
  normalizeProxyUrl,
  parseNoProxy,
  readProxyEnv,
} from '../../../src/lib/proxy/proxy-env'

describe('normalizeProxyUrl', () => {
  it('returns undefined for missing or blank values', () => {
    expect(normalizeProxyUrl(undefined)).toBeUndefined()
    expect(normalizeProxyUrl('')).toBeUndefined()
    expect(normalizeProxyUrl('   ')).toBeUndefined()
  })

  it('keeps URLs that already have a scheme', () => {
    expect(normalizeProxyUrl('http://proxy:8080')).toBe('http://proxy:8080')
    expect(normalizeProxyUrl('https://proxy:8443')).toBe('https://proxy:8443')
  })

  it('defaults bare host:port to http://', () => {
    expect(normalizeProxyUrl('proxy:8080')).toBe('http://proxy:8080')
  })
})

describe('parseNoProxy', () => {
  it('splits on commas and drops empties', () => {
    expect(parseNoProxy('localhost, 127.0.0.1,,*.corp')).toEqual([
      'localhost',
      '127.0.0.1',
      '*.corp',
    ])
  })

  it('returns an empty list for missing values', () => {
    expect(parseNoProxy(undefined)).toEqual([])
  })
})

describe('readProxyEnv', () => {
  it('prefers uppercase names and parses the bypass list', () => {
    expect(
      readProxyEnv({
        HTTP_PROXY: 'http://proxy:8080',
        http_proxy: 'http://other:8080',
        NO_PROXY: 'localhost,.corp',
      } as NodeJS.ProcessEnv),
    ).toEqual({
      httpProxy: 'http://proxy:8080',
      httpsProxy: undefined,
      noProxy: ['localhost', '.corp'],
    })
  })

  it('falls back to lowercase names', () => {
    expect(
      readProxyEnv({
        https_proxy: 'proxy:8443',
      } as NodeJS.ProcessEnv),
    ).toEqual({
      httpProxy: undefined,
      httpsProxy: 'http://proxy:8443',
      noProxy: [],
    })
  })

  it('returns empty settings for a blank env', () => {
    expect(readProxyEnv({} as NodeJS.ProcessEnv)).toEqual({
      httpProxy: undefined,
      httpsProxy: undefined,
      noProxy: [],
    })
  })
})
