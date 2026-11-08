// context.cookies through the pinned Playwright and through Neo, on fresh tabs of an inline page
// (events.mjs): a cookie the page writes reads back with Playwright's defaults (host-only domain,
// path /, expires -1 for a session cookie, SameSite Lax where the browser reports none), a dated
// one with the browser's 400-day cap; Set-Cookie's HttpOnly, SameSite and empty values read back
// as sent; and cookies(urls) returns just the cookies each URL would carry, by host, subdomain,
// path and scheme (server/network.ts `filterCookies`), a secure one included for http 127.0.0.1.
//
// Neo's context is the user's own profile, so a script reads only the cookies it named
// (`neo-co-`), from cookies() or cookies(urls) alike, and removes them before it returns. Other
// hosts are subdomains of the reserved example.com.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, events.mjs).

import { test } from 'node:test'
import { compareEvents } from './events.mjs'
import { skip } from './harness.mjs'

/** `ours(cookies)`: the case's cookies in a stable order; `forget(...)`: removes them by name. */
const COOKIES = `const origin = page.url().match(/^[a-z]+:\\/\\/[^/]+/)[0]
    const ours = (cookies) => cookies.filter((cookie) => cookie.name.startsWith('neo-co-'))
      .sort((a, b) => (a.name + ' ' + a.domain < b.name + ' ' + b.domain ? -1 : 1))
    const forget = async (...names) => {
      for (const name of names) await page.context().clearCookies({ name })
    }
    `

const PROBE = `document.cookie.split('; ').filter((pair) => pair.startsWith('neo-co-')).join('; ')`

const session = {
  domain: '127.0.0.1',
  path: '/',
  expires: -1,
  httpOnly: false,
  secure: false,
  sameSite: 'Lax',
}

const cases = [
  {
    // tests/library/browsercontext-cookies.spec.ts "CO-002 should get a cookie @smoke", "CO-003
    // should get a non-session cookie", "CO-008 should get multiple cookies".
    name: "cookies the page writes read back with Playwright's defaults, a dated one capped at 400 days",
    path: '/cookies.html',
    source: `${COOKIES}try {
      const documentCookie = await page.evaluate(() => {
        document.cookie = 'neo-co-username=John Doe'
        document.cookie = 'neo-co-dated=1; expires=' + new Date(Date.UTC(2038, 0, 1)).toUTCString()
        return document.cookie.split('; ').filter((pair) => pair.startsWith('neo-co-')).sort()
      })
      const now = Date.now() / 1000
      const [dated, username] = ours(await page.context().cookies())
      const day = 24 * 3600
      return {
        documentCookie,
        username,
        dated: { ...dated, expires: dated.expires > now + 399 * day && dated.expires < now + 401 * day },
      }
    } finally {
      await forget('neo-co-username', 'neo-co-dated')
    }`,
    expected: {
      documentCookie: ['neo-co-dated=1', 'neo-co-username=John Doe'],
      username: { name: 'neo-co-username', value: 'John Doe', ...session },
      dated: { name: 'neo-co-dated', value: '1', ...session, expires: true },
    },
    probe: PROBE,
    effect: '',
  },
  {
    // browsercontext-cookies.spec.ts "CO-005 should properly report httpOnly cookie", "CO-006
    // should properly report "Strict" sameSite cookie", "CO-007 should properly report "Lax"
    // sameSite cookie", "CO-012 should return cookies with empty value".
    name: "Set-Cookie's HttpOnly, SameSite and empty values read back as the server sent them",
    path: '/cookies.html',
    source: `${COOKIES}const start = page.url()
    try {
      await page.goto(origin + '/set-cookies')
      return ours(await page.context().cookies(origin + '/'))
    } finally {
      await forget('neo-co-httponly', 'neo-co-strict', 'neo-co-lax', 'neo-co-empty')
      await page.goto(start)
    }`,
    expected: [
      { name: 'neo-co-empty', value: '', ...session },
      { name: 'neo-co-httponly', value: 'value', ...session, httpOnly: true },
      { name: 'neo-co-lax', value: 'value', ...session },
      { name: 'neo-co-strict', value: 'value', ...session, sameSite: 'Strict' },
    ],
    probe: PROBE,
    effect: '',
  },
  {
    // browsercontext-cookies.spec.ts "CO-009 should get cookies from multiple urls", "CO-010
    // should work with subdomain cookie", "CO-011 should only return cookies that apply to the
    // url", "CO-013 should return secure cookies based on HTTP(S) protocol", "CO-014 should
    // return secure cookies for http 127.0.0.1".
    name: 'cookies(urls) returns the cookies each URL would carry, by host, subdomain, path and scheme',
    path: '/cookies.html',
    source: `${COOKIES}const host = 'neo-parity.example.com'
    const names = async (urls) => ours(await page.context().cookies(urls)).map((cookie) => cookie.name.slice(7))
    try {
      await page.context().addCookies([
        { name: 'neo-co-hostonly', value: 'v', domain: host, path: '/api' },
        { name: 'neo-co-subdomains', value: 'v', domain: '.' + host, path: '/api', sameSite: 'Lax' },
        { name: 'neo-co-slash', value: 'v', domain: host, path: '/foo/' },
        { name: 'neo-co-secure', value: 'v', domain: host, path: '/', secure: true },
        { name: 'neo-co-local', value: 'v', domain: '127.0.0.1', path: '/', secure: true },
      ])
      return {
        api: await names('http://' + host + '/api'),
        apiChild: await names('http://' + host + '/api/x'),
        apiv2: await names('http://' + host + '/apiv2'),
        subdomain: await names('http://sub.' + host + '/api'),
        anotherHost: await names('http://not' + host + '/api'),
        foo: await names('http://' + host + '/foo'),
        fooChild: await names('http://' + host + '/foo/bar'),
        https: await names('https://' + host + '/'),
        http: await names('http://' + host + '/'),
        several: await names(['https://' + host + '/api', 'http://sub.' + host + '/api']),
        localHttp: await names(origin + '/'),
        subdomainCookie: ours(await page.context().cookies('https://sub.' + host + '/api'))[0],
      }
    } finally {
      await forget('neo-co-hostonly', 'neo-co-subdomains', 'neo-co-slash', 'neo-co-secure', 'neo-co-local')
    }`,
    expected: {
      api: ['hostonly', 'subdomains'],
      apiChild: ['hostonly', 'subdomains'],
      apiv2: [],
      subdomain: ['subdomains'],
      anotherHost: [],
      foo: [],
      fooChild: ['slash'],
      https: ['secure'],
      http: [],
      several: ['hostonly', 'secure', 'subdomains'],
      localHttp: ['local'],
      subdomainCookie: {
        name: 'neo-co-subdomains',
        value: 'v',
        ...session,
        domain: '.neo-parity.example.com',
        path: '/api',
      },
    },
    probe: PROBE,
    effect: '',
  },
]

test('context.cookies reads the same cookies through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareEvents(t, {
    pages: {
      '/cookies.html': '<title>cookies</title><h1>cookies</h1>',
      '/set-cookies': {
        headers: {
          'set-cookie': [
            'neo-co-httponly=value; HttpOnly; Path=/',
            'neo-co-strict=value; SameSite=Strict; Path=/',
            'neo-co-lax=value; SameSite=Lax; Path=/',
            'neo-co-empty=; Path=/',
          ],
        },
        body: '<title>set-cookies</title><h1>set-cookies</h1>',
      },
    },
    cases,
  })
})
