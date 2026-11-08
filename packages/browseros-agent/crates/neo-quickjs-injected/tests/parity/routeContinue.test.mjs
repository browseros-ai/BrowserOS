// route.continue: a continued request goes to the network with the overrides it was given,
// through the pinned Playwright and through Neo (network.mjs). Cases from the pinned tests
// (26137face):
//
// - tests/page/page-request-continue.spec.ts RO-058 "should amend HTTP headers" and RO-069
//   "should amend method and post data": the server sees the header, the method and the body.
// - RO-063 "should not allow changing protocol when overriding url": the continue rejects, and
//   it used the route's one answer.
// - RO-083 "continue should propagate headers to redirects", with tests/page/page-route.spec.ts
//   RO-020 "should not work with redirects": only the first hop is routed, and the redirect's hop
//   still carries the header.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, network.mjs).

import { test } from 'node:test'
import { skip } from './harness.mjs'
import { compareNetwork } from './network.mjs'

const cases = [
  {
    name: 'the server sees the headers, method and body a continue gives',
    source: `await page.goto(ORIGIN + '/net/blank.html')
    await page.route('**/net/headers', (route) => route.continue({
      headers: { ...route.request().headers(), FOO: 'bar' },
      method: 'POST',
      postData: 'doggo',
    }))
    const seen = await page.evaluate(() => fetch('/net/headers').then((r) => r.json()))
    return { foo: seen.headers.foo, method: seen.method, body: seen.body }`,
    expected: { foo: 'bar', method: 'POST', body: 'doggo' },
  },
  {
    // The pinned test checks only the rejection: Playwright's dispatcher took the route's one
    // answer before the check, so a second answer rejects too, and the request stays paused.
    name: "a protocol change is refused, and it used the route's one answer",
    source: `let done
    const outcome = new Promise((resolve) => (done = resolve))
    await page.route('**/net/blank.html', async (route) => {
      const rejected = await settle(route.continue({ url: 'file:///tmp/foo' }))
      const second = await settle(route.abort())
      done({ rejected, second })
    })
    page.goto(ORIGIN + '/net/blank.html').catch(() => {})
    return await outcome`,
    expected: { rejected: { error: 'Error' }, second: { error: 'Error' } },
  },
  {
    name: "a redirect's hop keeps the headers its first hop was continued with",
    source: `await page.goto(ORIGIN + '/net/blank.html')
    const routed = []
    await page.route(/\\/net\\/(redirect|headers)/, (route) => {
      routed.push(path(route.request().url()))
      return route.continue({ headers: { ...route.request().headers(), custom: 'value' } })
    })
    const seen = await page.evaluate(() => fetch('/net/redirect?to=/net/headers').then((r) => r.json()))
    return { routed, custom: seen.headers.custom, method: seen.method }`,
    expected: {
      routed: ['/net/redirect?to=/net/headers'],
      custom: 'value',
      method: 'GET',
    },
  },
]

test('route.continue matches Playwright', {
  skip,
  timeout: 240_000,
}, async (t) => {
  await compareNetwork(t, cases)
})
