import { afterEach, describe, expect, test } from 'bun:test'
import { rmSync } from 'node:fs'
import {
  expiryToMs,
  type FirefoxCookie,
  originAttributesSkipReason,
  readCookieDatabase,
} from '../src/firefox/cookies'
import {
  decodeMozLz4,
  lz4BlockDecompress,
  readSessionCookies,
} from '../src/firefox/sessionstore'
import { hostMatches, paramKey, toCookieParam } from '../src/neo/cookies'
import {
  mozLz4,
  tempProfile,
  writeCookieDb,
  writeSessionStore,
} from './fixtures'

const NOW = 1_800_000_000_000
const dirs: string[] = []
function profile() {
  const dir = tempProfile()
  dirs.push(dir)
  return dir
}
afterEach(() => {
  for (const dir of dirs.splice(0))
    rmSync(dir, { recursive: true, force: true })
})

describe('readCookieDatabase', () => {
  test('keeps the default jar and reports the rest by reason', () => {
    const dir = profile()
    writeCookieDb(dir, [
      {
        name: 'sid',
        value: 'v1',
        host: '.example.com',
        expiry: NOW + 1000,
        isSecure: 1,
        isHttpOnly: 1,
        sameSite: 1,
      },
      {
        name: 'old',
        value: 'v2',
        host: 'example.com',
        expiry: (NOW - 1000) / 1000,
      },
      {
        name: 'c',
        value: 'v3',
        host: 'example.com',
        expiry: NOW + 1000,
        originAttributes: '^userContextId=2',
      },
      {
        name: 'p',
        value: 'v4',
        host: 'cdn.example',
        expiry: NOW + 1000,
        originAttributes: '^partitionKey=%28https%2Cexample.com%29',
      },
      {
        name: 'pb',
        value: 'v5',
        host: 'example.com',
        expiry: NOW + 1000,
        originAttributes: '^privateBrowsingId=1',
      },
    ])
    const result = readCookieDatabase(dir, NOW)
    expect(result.cookies).toEqual([
      {
        name: 'sid',
        value: 'v1',
        host: '.example.com',
        path: '/',
        expiresMs: NOW + 1000,
        secure: true,
        httpOnly: true,
        sameSite: 'Lax',
        schemeMap: 2,
      },
    ])
    expect(result.skipped).toEqual({
      expired: 1,
      container: 1,
      partitioned: 1,
      private: 1,
    })
  })

  test('expiry: seconds (schema < 16) and milliseconds (schema 17)', () => {
    expect(expiryToMs(1_800_000_000)).toBe(1_800_000_000_000)
    expect(expiryToMs(1_800_000_000_000)).toBe(1_800_000_000_000)
  })

  test('originAttributes classification', () => {
    expect(originAttributesSkipReason('')).toBeUndefined()
    expect(originAttributesSkipReason('^userContextId=0')).toBe(
      'origin-attributes',
    )
    expect(originAttributesSkipReason('^firstPartyDomain=example.com')).toBe(
      'origin-attributes',
    )
  })
})

describe('session store', () => {
  test('decodes LZ4 matches that overlap their own output', () => {
    // token: 3 literals, match length 5+4 at offset 3
    const block = Uint8Array.from([0x35, 0x61, 0x62, 0x63, 0x03, 0x00])
    expect(Buffer.from(lz4BlockDecompress(block, 12)).toString()).toBe(
      'abcabcabcabc',
    )
  })

  test('decodes mozLz4 with extended literal lengths', () => {
    const text = 'x'.repeat(300)
    expect(decodeMozLz4(mozLz4(text))).toBe(text)
    expect(() => decodeMozLz4(Buffer.from('not lz4 at all'))).toThrow(
      'Not a mozLz4',
    )
  })

  test('reads session-only cookies of the default jar', () => {
    const dir = profile()
    writeSessionStore(dir, {
      windows: [],
      cookies: [
        {
          host: 'app.example.com',
          name: 'JSESSIONID',
          value: 's1',
          path: '/app',
          secure: true,
          httponly: true,
          schemeMap: 2,
        },
        {
          host: '.example.com',
          name: 'c',
          value: 's2',
          originAttributes: { userContextId: 1 },
        },
      ],
    })
    const result = readSessionCookies(dir)
    expect(result.cookies).toEqual([
      {
        name: 'JSESSIONID',
        value: 's1',
        host: 'app.example.com',
        path: '/app',
        expiresMs: null,
        secure: true,
        httpOnly: true,
        sameSite: undefined,
        schemeMap: 2,
      },
    ])
    expect(result.skipped).toEqual({ container: 1 })
  })

  test('a profile without a session file yields nothing', () => {
    expect(readSessionCookies(profile())).toEqual({ cookies: [], skipped: {} })
  })
})

describe('toCookieParam', () => {
  const base: FirefoxCookie = {
    name: 'n',
    value: 'v',
    host: 'example.com',
    path: '/',
    expiresMs: 1_800_000_000_500,
    secure: false,
    httpOnly: false,
    schemeMap: 1,
  }

  test('host-only cookies use a URL so they do not widen to subdomains', () => {
    const param = toCookieParam(base)
    expect(param).toEqual({
      name: 'n',
      value: 'v',
      url: 'http://example.com/',
      path: '/',
      secure: false,
      httpOnly: false,
      expires: 1_800_000_000.5,
    })
    expect(param.domain).toBeUndefined()
    expect(paramKey(param)).toBe('n\u0000example.com\u0000/')
  })

  test('domain cookies keep the leading dot', () => {
    const param = toCookieParam({
      ...base,
      host: '.example.com',
      secure: true,
      sameSite: 'Strict',
    })
    expect(param.domain).toBe('.example.com')
    expect(param.url).toBeUndefined()
    expect(param.sameSite).toBe('Strict')
  })

  test('SameSite=None survives only on Secure cookies', () => {
    expect(
      toCookieParam({ ...base, sameSite: 'None' }).sameSite,
    ).toBeUndefined()
    expect(
      toCookieParam({ ...base, secure: true, sameSite: 'None' }).sameSite,
    ).toBe('None')
  })

  test('session cookies carry no expiry', () => {
    expect(toCookieParam({ ...base, expiresMs: null }).expires).toBeUndefined()
  })

  test('https is used for secure or https-scheme cookies', () => {
    expect(toCookieParam({ ...base, schemeMap: 2 }).url).toBe(
      'https://example.com/',
    )
    expect(toCookieParam({ ...base, secure: true }).url).toBe(
      'https://example.com/',
    )
  })
})

test('hostMatches limits to a domain and its subdomains', () => {
  expect(hostMatches('.mail.example.com', ['example.com'])).toBe(true)
  expect(hostMatches('example.com', ['.example.com'])).toBe(true)
  expect(hostMatches('notexample.com', ['example.com'])).toBe(false)
  expect(hostMatches('anything.org', [])).toBe(true)
})
