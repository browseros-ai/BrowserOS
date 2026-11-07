// APIRequestContext.storageState: the browser context's request context gives its context's
// state, and a newContext() gives back the state it was made with, through the pinned Playwright
// and through Neo (network.mjs). Cases from the pinned tests (26137face):
//
// - tests/library/browsercontext-fetch.spec.ts FE-165 "context request should export same storage
//   state as context".
// - tests/library/global-fetch-cookie.spec.ts FE-084 "should preserve origin storage on
//   import/export of storage state", without IndexedDB.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, network.mjs).

import { test } from 'node:test'
import { skip } from './harness.mjs'
import { compareNetwork } from './network.mjs'

const cases = [
  {
    name: "context.request's state is its context's",
    source: `await context.clearCookies({ domain: '127.0.0.1' })
    await page.goto(ORIGIN + '/net/blank.html')
    await page.request.get(ORIGIN + '/net/set-cookie?cookie=' + encodeURIComponent('state=1; Path=/'))
    const fromRequest = await page.request.storageState()
    const fromContext = await context.storageState()
    await context.clearCookies({ domain: '127.0.0.1' })
    return {
      same: JSON.stringify(fromRequest) === JSON.stringify(fromContext),
      cookie: fromRequest.cookies.filter((cookie) => cookie.domain === '127.0.0.1').map((cookie) => [cookie.name, cookie.value]),
    }`,
    expected: { same: true, cookie: [['state', '1']] },
  },
  {
    name: 'a newContext gives back the state it was made with',
    source: `const state = {
      cookies: [{ name: 'a', value: 'b', domain: '127.0.0.1', path: '/', expires: -1, httpOnly: false, secure: false, sameSite: 'Lax' }],
      origins: [{ origin: ORIGIN, localStorage: [{ name: 'k', value: 'v' }] }],
    }
    const request = await playwright.request.newContext({ storageState: state })
    const back = await request.storageState()
    const sent = await (await request.get(ORIGIN + '/net/cookies')).json()
    await request.dispose()
    return { back, sent }`,
    expected: {
      back: {
        cookies: [
          {
            name: 'a',
            value: 'b',
            domain: '127.0.0.1',
            path: '/',
            expires: -1,
            httpOnly: false,
            secure: false,
            sameSite: 'Lax',
          },
        ],
        origins: [
          {
            origin: 'ORIGIN',
            localStorage: [{ name: 'k', value: 'v' }],
            indexedDB: [],
          },
        ],
      },
      sent: { cookie: 'a=b' },
    },
  },
]

// The origin's port changes between runs: the expectation names it ORIGIN.
for (const item of cases)
  if (item.name.startsWith('a newContext'))
    item.source = item.source.replace(
      'return { back, sent }',
      "back.origins = back.origins.map((origin) => ({ ...origin, origin: origin.origin === ORIGIN ? 'ORIGIN' : origin.origin }))\n    return { back, sent }",
    )

test('APIRequestContext.storageState matches Playwright', {
  skip,
  timeout: 240_000,
}, async (t) => {
  await compareNetwork(t, cases)
})
