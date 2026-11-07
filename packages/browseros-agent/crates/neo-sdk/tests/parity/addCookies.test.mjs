// context.addCookies through the pinned Playwright and through Neo, on fresh tabs of an inline
// page (events.mjs): cookies added for the page's URL reach document.cookie and the requests the
// server sees, and read back as Playwright derives them from a URL (server/network.ts
// `rewriteCookies`: the URL's host, its directory as the path, secure from its scheme) with a
// session expiry and SameSite Lax; cookies added by domain and path, with an expiry, HttpOnly,
// Secure and SameSite of their own, read back as added and reach only the requests under their
// path; a batch holding one cookie Playwright refuses rejects with Error and adds none of it.
//
// Neo's context is the user's own profile, so a script reads only the cookies it named
// (`neo-ac-`), filtering cookies() by the fixture's URLs, and removes them before it returns.
// The server echoes each request's Cookie header, filtered the same way.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, events.mjs).

import { test } from 'node:test'
import { compareEvents } from './events.mjs'
import { skip } from './harness.mjs'

/**
 * `ours(cookies)`: the case's cookies in a stable order; `seen()`: the page's document.cookie
 * pairs; `sent(path)`: the pairs of the Cookie header a fetch of `path` carried; `forget(...)`:
 * removes cookies by name.
 */
const COOKIES = `const origin = page.url().match(/^[a-z]+:\\/\\/[^/]+/)[0]
    const mine = (pair) => pair.startsWith('neo-ac-')
    const ours = (cookies) => cookies.filter((cookie) => mine(cookie.name))
      .sort((a, b) => (a.name + ' ' + a.path < b.name + ' ' + b.path ? -1 : 1))
    const seen = async () => (await page.evaluate(() => document.cookie)).split('; ').filter(mine).sort()
    const sent = async (path) => (await page.evaluate(async (path) => (await fetch(path)).text(), path))
      .split('; ').filter(mine).sort()
    const forget = async (...names) => {
      for (const name of names) await page.context().clearCookies({ name })
    }
    `

const PROBE = `document.cookie.split('; ').filter((pair) => pair.startsWith('neo-ac-')).join('; ')`

/** Answers with the request's Cookie header. */
function cookieHeader(request, response) {
  response.setHeader('content-type', 'text/plain; charset=utf-8')
  response.end(request.headers.cookie ?? '')
}

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
    // tests/library/browsercontext-add-cookies.spec.ts "CO-019 should work @smoke", "CO-030
    // should set multiple cookies", "CO-024 should send cookie header", "CO-032 should set cookie
    // with reasonable defaults", "CO-031 should have |expires| set to |-1| for session cookies".
    name: "cookies added for the page's URL reach the page and the server, with Playwright's defaults",
    path: '/addCookies.html',
    source: `${COOKIES}try {
      await page.context().addCookies([
        { url: origin + '/addCookies.html', name: 'neo-ac-password', value: '123456' },
        { url: origin + '/', name: 'neo-ac-multiple', value: 'bar' },
      ])
      return {
        documentCookie: await seen(),
        sent: await sent('/cookie-header'),
        stored: ours(await page.context().cookies(origin + '/')),
      }
    } finally {
      await forget('neo-ac-password', 'neo-ac-multiple')
    }`,
    expected: {
      documentCookie: ['neo-ac-multiple=bar', 'neo-ac-password=123456'],
      sent: ['neo-ac-multiple=bar', 'neo-ac-password=123456'],
      stored: [
        { name: 'neo-ac-multiple', value: 'bar', ...session },
        { name: 'neo-ac-password', value: '123456', ...session },
      ],
    },
    probe: PROBE,
    effect: '',
  },
  {
    // browsercontext-add-cookies.spec.ts "CO-033 should set a cookie with a path", "CO-036
    // should default to setting secure cookie for HTTPS websites", "CO-038 should set a cookie
    // on a different domain"; tests/library/browsercontext-cookies.spec.ts "CO-015 should add
    // cookies with an expiration". A URL's cookie takes the URL's directory as its path.
    name: "a cookie's path is its URL's directory or its own, and only requests under it carry it",
    path: '/addCookies.html',
    source: `${COOKIES}const expires = Math.floor(Date.now() / 1000) + 3600
    try {
      await page.context().addCookies([
        { url: origin + '/dir/page.html', name: 'neo-ac-directory', value: 'url' },
        { url: 'https://neo-parity.example.com/', name: 'neo-ac-https', value: 'secure' },
        { domain: '127.0.0.1', path: '/grid.html', name: 'neo-ac-grid', value: 'GRID', sameSite: 'Strict' },
        {
          domain: '127.0.0.1', path: '/', name: 'neo-ac-dated', value: 'x',
          expires, httpOnly: true, secure: true, sameSite: 'None',
        },
      ])
      const stored = ours(await page.context().cookies([
        origin + '/dir/', origin + '/grid.html', 'https://neo-parity.example.com/',
      ]))
      return {
        stored: stored.map((cookie) => ({ ...cookie, expires: cookie.expires === expires ? 'as added' : cookie.expires })),
        documentCookie: await seen(),
        sent: {
          root: await sent('/cookie-header'),
          directory: await sent('/dir/cookie-header'),
          grid: await sent('/grid.html'),
        },
      }
    } finally {
      await forget('neo-ac-directory', 'neo-ac-https', 'neo-ac-grid', 'neo-ac-dated')
    }`,
    expected: {
      stored: [
        {
          name: 'neo-ac-dated',
          value: 'x',
          domain: '127.0.0.1',
          path: '/',
          expires: 'as added',
          httpOnly: true,
          secure: true,
          sameSite: 'None',
        },
        { name: 'neo-ac-directory', value: 'url', ...session, path: '/dir/' },
        {
          name: 'neo-ac-grid',
          value: 'GRID',
          ...session,
          path: '/grid.html',
          sameSite: 'Strict',
        },
        {
          name: 'neo-ac-https',
          value: 'secure',
          ...session,
          domain: 'neo-parity.example.com',
          secure: true,
        },
      ],
      documentCookie: [],
      sent: {
        root: ['neo-ac-dated=x'],
        directory: ['neo-ac-dated=x', 'neo-ac-directory=url'],
        grid: ['neo-ac-dated=x', 'neo-ac-grid=GRID'],
      },
    },
    probe: PROBE,
    effect: '',
  },
  {
    // browsercontext-add-cookies.spec.ts "CO-034 should not set a cookie with blank page URL",
    // "CO-035 should not set a cookie on a data URL page"; browsercontext-cookies.spec.ts "CO-015
    // should add cookies with an expiration" (a negative expiry and one past 9999 reject), and
    // rewriteCookies' other checks: a cookie needs a url or a domain and path, not both.
    name: 'a batch holding a cookie Playwright refuses rejects with Error and adds none of it',
    path: '/addCookies.html',
    source: `${COOKIES}const good = { url: origin + '/', name: 'neo-ac-good', value: '1' }
    const add = (cookie) => settle(page.context().addCookies([good, cookie]))
    try {
      const refused = {
        blank: await add({ url: 'about:blank', name: 'neo-ac-blank', value: '1' }),
        data: await add({ url: 'data:,Hello%2C%20World!', name: 'neo-ac-data', value: '1' }),
        negativeExpiry: await add({ url: origin + '/', name: 'neo-ac-expiry', value: '1', expires: -42 }),
        pastYear9999: await add({ url: origin + '/', name: 'neo-ac-expiry', value: '1', expires: 253402300800 }),
        noUrlNorDomain: await add({ name: 'neo-ac-nowhere', value: '1' }),
        urlAndDomain: await add({ url: origin + '/', domain: '127.0.0.1', name: 'neo-ac-both', value: '1' }),
      }
      return { refused, stored: ours(await page.context().cookies(origin + '/')).map((cookie) => cookie.name) }
    } finally {
      await forget('neo-ac-good')
    }`,
    expected: {
      refused: {
        blank: { error: 'Error' },
        data: { error: 'Error' },
        negativeExpiry: { error: 'Error' },
        pastYear9999: { error: 'Error' },
        noUrlNorDomain: { error: 'Error' },
        urlAndDomain: { error: 'Error' },
      },
      stored: [],
    },
    probe: PROBE,
    effect: '',
  },
]

test('context.addCookies adds the same cookies through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareEvents(t, {
    pages: {
      '/addCookies.html': '<title>addCookies</title><h1>addCookies</h1>',
      '/cookie-header': cookieHeader,
      '/dir/cookie-header': cookieHeader,
      '/grid.html': cookieHeader,
    },
    cases,
  })
})
