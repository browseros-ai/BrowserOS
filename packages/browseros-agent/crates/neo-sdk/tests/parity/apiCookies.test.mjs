// The browser context's request context shares the context's cookies both ways, as Playwright's
// does: a response's Set-Cookie reaches the browser (and the page), and the browser's cookies go
// out with the request; through the pinned Playwright and through Neo (network.mjs). Cases from
// the pinned tests (26137face):
//
// - tests/library/browsercontext-fetch.spec.ts FE-107 "should add cookies from Set-Cookie header".
// - A cookie the page sets goes out with page.request (FE-096's direction, without its DNS hook).
// - FE-108 "page.request.addCookies should add cookies to the browser context".
//
// Both engines' turns share one browser: each case clears 127.0.0.1's cookies first and last.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, network.mjs).

import { test } from 'node:test'
import { skip } from './harness.mjs'
import { compareNetwork } from './network.mjs'

const CLEAR = `await context.clearCookies({ domain: '127.0.0.1' })`

const cases = [
  {
    name: "a response's Set-Cookie reaches the browser and the page",
    source: `${CLEAR}
    await page.goto(ORIGIN + '/net/blank.html')
    await page.request.get(ORIGIN + '/net/set-cookie?cookie=' + encodeURIComponent('a=b; Path=/'))
    const cookies = (await context.cookies()).filter((cookie) => cookie.domain === '127.0.0.1').map((cookie) => [cookie.name, cookie.value, cookie.path])
    const seen = await page.evaluate(() => document.cookie)
    ${CLEAR}
    return { cookies, seen }`,
    expected: { cookies: [['a', 'b', '/']], seen: 'a=b' },
  },
  {
    name: 'a cookie the page sets goes out with page.request',
    source: `${CLEAR}
    await page.goto(ORIGIN + '/net/blank.html')
    await page.evaluate(() => { document.cookie = 'x=1; path=/' })
    const response = await page.request.get(ORIGIN + '/net/cookies')
    const sent = await response.json()
    ${CLEAR}
    return sent`,
    expected: { cookie: 'x=1' },
  },
  {
    name: 'page.request.addCookies adds them to the browser context',
    source: `${CLEAR}
    await page.goto(ORIGIN + '/net/blank.html')
    await page.request.addCookies([{ name: 'z', value: '2', url: ORIGIN }])
    const inContext = (await context.cookies(ORIGIN)).map((cookie) => [cookie.name, cookie.value])
    const fromRequest = (await page.request.cookies(ORIGIN)).map((cookie) => [cookie.name, cookie.value])
    const sent = await (await page.request.get(ORIGIN + '/net/cookies')).json()
    const seen = await page.evaluate(() => document.cookie)
    ${CLEAR}
    return { inContext, fromRequest, sent, seen }`,
    expected: {
      inContext: [['z', '2']],
      fromRequest: [['z', '2']],
      sent: { cookie: 'z=2' },
      seen: 'z=2',
    },
  },
]

test('the browser context request shares cookies as Playwright', {
  skip,
  timeout: 240_000,
}, async (t) => {
  await compareNetwork(t, cases)
})
