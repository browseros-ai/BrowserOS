// APIRequestContext redirects, followed by the request context itself (Playwright's server, Neo's
// Rust client), through the pinned Playwright and through Neo (network.mjs). Cases from the pinned
// tests (26137face):
//
// - tests/library/global-fetch.spec.ts FE-032 "should remove content-length from redirected post
//   requests": a 302 turns a POST into a GET without its body.
// - A 307 keeps the method and body (the fetch spec's redirect step Playwright follows).
// - FE-039 "should throw an error when maxRedirects is exceeded", FE-040 "should not follow
//   redirects when maxRedirects is set to 0".
//
// Errors compare by their first line: Playwright's carry a call log after it.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, network.mjs).

import { test } from 'node:test'
import { skip } from './harness.mjs'
import { compareNetwork } from './network.mjs'

const cases = [
  {
    name: 'a 302 turns a POST into a GET without its body; a 307 keeps both',
    source: `await page.goto(ORIGIN + '/net/blank.html')
    const hop = async (status) => {
      const response = await page.request.post(ORIGIN + '/net/redirect?to=/net/headers&status=' + status, { data: 'hello' })
      const echoed = await response.json()
      return { url: path(response.url()), method: echoed.method, length: echoed.headers['content-length'] ?? null, body: echoed.body }
    }
    return { found: await hop(302), temporary: await hop(307) }`,
    expected: {
      found: { url: '/net/headers', method: 'GET', length: null, body: '' },
      temporary: {
        url: '/net/headers',
        method: 'POST',
        length: '5',
        body: 'hello',
      },
    },
  },
  {
    name: 'past maxRedirects the request fails; maxRedirects 0 returns the redirect',
    source: `await page.goto(ORIGIN + '/net/blank.html')
    const twice = ORIGIN + '/net/redirect?to=' + encodeURIComponent('/net/redirect?to=/net/data.json')
    const failed = await page.request.get(twice, { maxRedirects: 1 }).then(() => null, (error) => error.message.split('\\n')[0])
    const followed = await page.request.get(twice, { maxRedirects: 2 })
    const kept = await page.request.get(ORIGIN + '/net/redirect?to=/net/data.json', { maxRedirects: 0 })
    return {
      failed,
      followed: [path(followed.url()), followed.status()],
      kept: [kept.status(), kept.headers().location],
    }`,
    expected: {
      failed: 'apiRequestContext.get: Max redirect count exceeded',
      followed: ['/net/data.json', 200],
      kept: [302, '/net/data.json'],
    },
  },
]

test('APIRequestContext redirects match Playwright', {
  skip,
  timeout: 240_000,
}, async (t) => {
  await compareNetwork(t, cases)
})
