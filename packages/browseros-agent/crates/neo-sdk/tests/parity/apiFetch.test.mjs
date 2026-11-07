// APIRequestContext.fetch: the method a request goes with, and the bodies the response comes
// back with (a failing status, a gzipped one), through the pinned Playwright and through Neo
// (network.mjs). Cases from the pinned tests (26137face):
//
// - tests/library/global-fetch.spec.ts FE-038 "should return body for failing requests": HEAD and
//   PUT on a 404, the HEAD's body empty.
// - fetch's `method`, upper-cased whatever its case, with a body.
// - A gzipped response decodes, its content-encoding header kept.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, network.mjs).

import { test } from 'node:test'
import { skip } from './harness.mjs'
import { compareNetwork } from './network.mjs'

const cases = [
  {
    name: 'a failing request answers with its status and body',
    source: `await page.goto(ORIGIN + '/net/blank.html')
    const head = await page.request.fetch(ORIGIN + '/net/404', { method: 'HEAD' })
    const put = await page.request.fetch(ORIGIN + '/net/404', { method: 'PUT' })
    return { head: [head.status(), await head.text()], put: [put.status(), await put.text()] }`,
    expected: { head: [404, ''], put: [404, 'missing'] },
  },
  {
    name: "fetch's method goes upper-cased, with its body",
    source: `await page.goto(ORIGIN + '/net/blank.html')
    const echoed = await (await page.request.fetch(ORIGIN + '/net/headers', { method: 'patch', data: 'x' })).json()
    return [echoed.method, echoed.body]`,
    expected: ['PATCH', 'x'],
  },
  {
    name: 'a gzipped response decodes and keeps its content-encoding',
    source: `await page.goto(ORIGIN + '/net/blank.html')
    const response = await page.request.get(ORIGIN + '/net/gzip')
    return { encoding: response.headers()['content-encoding'], json: await response.json() }`,
    expected: { encoding: 'gzip', json: { zipped: true } },
  },
]

test('APIRequestContext.fetch matches Playwright', {
  skip,
  timeout: 240_000,
}, async (t) => {
  await compareNetwork(t, cases)
})
