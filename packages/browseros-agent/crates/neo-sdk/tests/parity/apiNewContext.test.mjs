// playwright.request.newContext(): a request context of its own (its options, its own cookie jar)
// and its dispose, through the pinned Playwright and through Neo (network.mjs). Cases from the
// pinned tests (26137face):
//
// - tests/library/global-fetch.spec.ts FE-003 "should support global userAgent option", FE-025
//   "should resolve url relative to global baseURL option".
// - tests/library/global-fetch-cookie.spec.ts: a Set-Cookie goes to the context's own jar and out
//   with its next request, never to the browser.
// - tests/library/global-fetch.spec.ts FE-175 "should not work after dispose", FE-030 "should
//   abort requests when context is disposed".
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, network.mjs).

import { test } from 'node:test'
import { skip } from './harness.mjs'
import { compareNetwork } from './network.mjs'

const cases = [
  {
    name: 'a context sends its userAgent and resolves against its baseURL',
    source: `const request = await playwright.request.newContext({ userAgent: 'My Agent', baseURL: ORIGIN + '/net/' })
    const echoed = await (await request.get('headers')).json()
    const data = await request.get('data.json')
    await request.dispose()
    return { agent: echoed.headers['user-agent'], url: path(data.url()) }`,
    expected: { agent: 'My Agent', url: '/net/data.json' },
  },
  {
    name: "a context's cookies are its own, sent on its next request",
    source: `await context.clearCookies({ domain: '127.0.0.1' })
    const request = await playwright.request.newContext()
    await request.get(ORIGIN + '/net/set-cookie?cookie=' + encodeURIComponent('own=1; Path=/'))
    const sent = await (await request.get(ORIGIN + '/net/cookies')).json()
    const jar = (await request.cookies()).map((cookie) => [cookie.name, cookie.value])
    const browser = (await context.cookies()).filter((cookie) => cookie.domain === '127.0.0.1').length
    await request.dispose()
    return { sent, jar, browser }`,
    expected: { sent: { cookie: 'own=1' }, jar: [['own', '1']], browser: 0 },
  },
  {
    name: 'dispose aborts what is in flight, and the context serves nothing after',
    source: `const request = await playwright.request.newContext()
    const held = request.get(ORIGIN + '/net/hold?id=api-dispose').then(() => null, (error) => [error.name, error.message.split('\\n')[0]])
    await new Promise((resolve) => setTimeout(resolve, 300))
    await request.dispose({ reason: 'done with it' })
    const inFlight = await held
    const after = await request.get(ORIGIN + '/net/data.json').then(() => null, (error) => [error.name, error.message.split('\\n')[0]])
    await page.goto(ORIGIN + '/net/release?id=api-dispose')
    return { inFlight, after }`,
    expected: {
      inFlight: ['Error', 'apiRequestContext.get: done with it'],
      after: ['Error', 'apiRequestContext.get: done with it'],
    },
  },
]

test('playwright.request.newContext matches Playwright', {
  skip,
  timeout: 240_000,
}, async (t) => {
  await compareNetwork(t, cases)
})
