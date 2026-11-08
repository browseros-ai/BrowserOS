// route.fallback: handlers run newest first, each passing the request on with what it changed,
// then the network, through the pinned Playwright and through Neo (network.mjs). Cases from the
// pinned tests (26137face):
//
// - tests/page/page-request-fallback.spec.ts RO-121 "should fall back": newest first, then the
//   network.
// - RO-127 "should amend HTTP headers": the next handler and the server see the header.
// - RO-123 "should not chain fulfill": a fulfill ends the chain.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, network.mjs).

import { test } from 'node:test'
import { skip } from './harness.mjs'
import { compareNetwork } from './network.mjs'

const cases = [
  {
    name: 'handlers fall back newest first, then the network answers',
    source: `const calls = []
    for (const n of [1, 2, 3])
      await page.route('**/net/blank.html', (route) => {
        calls.push(n)
        return route.fallback()
      })
    const response = await page.goto(ORIGIN + '/net/blank.html')
    return { calls, status: response.status(), title: await page.title() }`,
    expected: { calls: [3, 2, 1], status: 200, title: 'blank' },
  },
  {
    name: "a fallback's headers reach the next handler and the server",
    source: `await page.goto(ORIGIN + '/net/blank.html')
    const values = []
    await page.route('**/net/headers', async (route) => {
      values.push(route.request().headers().foo, await route.request().headerValue('FOO'))
      await route.continue()
    })
    await page.route('**/net/headers', (route) => route.fallback({ headers: { ...route.request().headers(), FOO: 'bar' } }))
    const seen = await page.evaluate(() => fetch('/net/headers').then((r) => r.json()))
    return [...values, seen.headers.foo]`,
    expected: ['bar', 'bar', 'bar'],
  },
  {
    name: 'a fulfill ends the chain',
    source: `let older = false
    await page.route('**/net/blank.html', () => {
      older = true
    })
    await page.route('**/net/blank.html', (route) => route.fulfill({ status: 200, body: 'fulfilled' }))
    await page.route('**/net/blank.html', (route) => route.fallback())
    const response = await page.goto(ORIGIN + '/net/blank.html')
    return { body: await response.text(), older }`,
    expected: { body: 'fulfilled', older: false },
  },
]

test('route.fallback matches Playwright', {
  skip,
  timeout: 240_000,
}, async (t) => {
  await compareNetwork(t, cases)
})
