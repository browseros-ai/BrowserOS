// APIRequestContext failOnStatusCode: a status past 399 fails the call with the status and the
// body's text, per call and per context, through the pinned Playwright and through Neo
// (network.mjs). Cases from the pinned tests (26137face):
//
// - tests/library/browsercontext-fetch.spec.ts FE-102 "<method> should support failOnStatusCode":
//   `404 Not Found`, then `Response text:` and the body (none for HEAD).
// - tests/library/global-fetch.spec.ts FE-052 "should throw when failOnStatusCode is set to true
//   inside APIRequest context options".
//
// Errors compare by their first lines: Playwright's carry a call log after them.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, network.mjs).

import { test } from 'node:test'
import { skip } from './harness.mjs'
import { compareNetwork } from './network.mjs'

const cases = [
  {
    name: 'a 404 fails get with its text and head without one',
    source: `await page.goto(ORIGIN + '/net/blank.html')
    const lines = (error, count) => error.message.split('\\n').slice(0, count)
    const get = await page.request.get(ORIGIN + '/net/404', { failOnStatusCode: true }).then(() => null, (error) => lines(error, 3))
    const head = await page.request.head(ORIGIN + '/net/404', { failOnStatusCode: true }).then(() => null, (error) => lines(error, 1))
    const passed = await page.request.get(ORIGIN + '/net/404')
    return { get, head, passed: passed.status() }`,
    expected: {
      get: [
        'apiRequestContext.get: 404 Not Found',
        'Response text:',
        'missing',
      ],
      head: ['apiRequestContext.head: 404 Not Found'],
      passed: 404,
    },
  },
  {
    name: "a context's failOnStatusCode fails its requests",
    source: `const request = await playwright.request.newContext({ failOnStatusCode: true })
    const failed = await request.get(ORIGIN + '/net/404').then(() => null, (error) => error.message.split('\\n')[0])
    const ok = await request.get(ORIGIN + '/net/data.json')
    await request.dispose()
    return { failed, ok: ok.status() }`,
    expected: { failed: 'apiRequestContext.get: 404 Not Found', ok: 200 },
  },
]

test('APIRequestContext failOnStatusCode matches Playwright', {
  skip,
  timeout: 240_000,
}, async (t) => {
  await compareNetwork(t, cases)
})
