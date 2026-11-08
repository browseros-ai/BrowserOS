// route.fetch: a handler reads the routed request's response, then answers with it, through the
// pinned Playwright (which fetches from Node) and through Neo (whose browser sends the routed
// request itself, neo-runtime routes.rs). Only the response's status, text, url, content type
// and body are compared: transport headers differ between Node and the browser. Cases from the
// pinned tests (26137face):
//
// - tests/page/page-request-intercept.spec.ts RO-142 "should fulfill intercepted response using
//   alias" and RO-139 "should give access to the intercepted response".
// - RO-144 "should not follow redirects when maxRedirects is set to 0 in route.fetch".
// - RO-143 "should support timeout option in route.fetch": the fetch times out, and the route
//   still answers.
// - The B3 review's case: a fetch past a nonzero redirect limit rejects, and the route still
//   fulfills its request.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, network.mjs).

import { test } from 'node:test'
import { skip } from './harness.mjs'
import { compareNetwork } from './network.mjs'

const cases = [
  {
    name: 'a fetched response reads as the server sent it and fulfills the request',
    source: `let fetched
    await page.route('**/net/blank.html', async (route) => {
      const response = await route.fetch()
      fetched = [
        response.status(),
        response.statusText(),
        response.ok(),
        path(response.url()),
        response.headers()['content-type'],
        await response.text(),
      ]
      await route.fulfill({ response })
    })
    const response = await page.goto(ORIGIN + '/net/blank.html')
    return { fetched, status: response.status(), type: response.headers()['content-type'], title: await page.title() }`,
    expected: {
      fetched: [
        200,
        'OK',
        true,
        '/net/blank.html',
        'text/html; charset=utf-8',
        '<title>blank</title>',
      ],
      status: 200,
      type: 'text/html; charset=utf-8',
      title: 'blank',
    },
  },
  {
    name: 'maxRedirects: 0 answers the redirect itself',
    source: `let fetched
    await page.route('**/net/redirect', async (route) => {
      const response = await route.fetch({ maxRedirects: 0 })
      fetched = [response.status(), response.headers().location]
      await route.fulfill({ contentType: 'text/html', body: 'hello' })
    })
    await page.goto(ORIGIN + '/net/redirect')
    return { fetched, shown: await page.evaluate(() => document.body.textContent) }`,
    expected: { fetched: [302, '/net/data.json'], shown: 'hello' },
  },
  {
    // B3 review (review-review-b3.md, P2): a fetch past a nonzero redirect limit rejects, and the
    // route still answers its request: the recovery fulfill is what the page reads.
    name: 'a fetch past its redirect limit rejects, and the route still fulfills',
    source: `await page.goto(ORIGIN + '/net/blank.html')
    let done
    const handled = new Promise((resolve) => (done = resolve))
    await page.route(/\\/net\\/redirect\\?to=%2F/, async (route) => {
      let fetchRejected = false
      let fulfilled = false
      try {
        await route.fetch({ maxRedirects: 1, timeout: 5000 })
      } catch {
        fetchRejected = true
      }
      try {
        await route.fulfill({ body: 'recovered' })
        fulfilled = true
      } catch {}
      done({ fetchRejected, fulfilled })
    })
    await page.evaluate(() => {
      // Two redirects: the first followed, the second past the limit.
      window.redirectBody = fetch('/net/redirect?to=' + encodeURIComponent('/net/redirect?to=/net/data.json')).then((r) => r.text())
    })
    const recovery = await handled
    await page.unrouteAll()
    const body = await page.evaluate(() => window.redirectBody)
    return { recovery, body }`,
    expected: {
      recovery: { fetchRejected: true, fulfilled: true },
      body: 'recovered',
    },
  },
  {
    name: 'a fetch past its timeout rejects, and the route still answers',
    source: `await page.goto(ORIGIN + '/net/blank.html')
    let fetchResult
    await page.route(/\\/net\\/hold\\?id=route-fetch$/, async (route) => {
      fetchResult = await settle(route.fetch({ timeout: 1000 }))
      await route.fulfill({ body: 'after timeout' })
    })
    const body = await page.evaluate(() => fetch('/net/hold?id=route-fetch').then((r) => r.text()))
    await page.evaluate(() => fetch('/net/release?id=route-fetch').then((r) => r.text()))
    return { fetchResult, body }`,
    expected: { fetchResult: { error: 'TimeoutError' }, body: 'after timeout' },
  },
]

test('route.fetch matches Playwright', {
  skip,
  timeout: 240_000,
}, async (t) => {
  await compareNetwork(t, cases)
})
