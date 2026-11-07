// page.request.get: an API request through the browser context's request context, sent from
// outside the browser (Playwright: Node; Neo: Rust, neo-runtime `request_contexts.rs`), through
// the pinned Playwright and through Neo (network.mjs). Cases from the pinned tests (26137face):
//
// - tests/library/browsercontext-fetch.spec.ts FE-088 "get should work @smoke": the response's
//   url, status, statusText, ok, headers and body.
// - FE-098 "get should support params passed as object": params, Cyrillic included, reach the
//   server and the response's url.
// - FE-152 "should dispose": a disposed response's body is gone; `page.request` is
//   `context.request`.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, network.mjs).

import { test } from 'node:test'
import { skip } from './harness.mjs'
import { compareNetwork } from './network.mjs'

const cases = [
  {
    name: 'get reads the response as the server sent it',
    source: `await page.goto(ORIGIN + '/net/blank.html')
    const response = await page.request.get(ORIGIN + '/net/data.json')
    return {
      url: path(response.url()),
      status: response.status(),
      statusText: response.statusText(),
      ok: response.ok(),
      contentType: response.headers()['content-type'],
      text: await response.text(),
      json: await response.json(),
    }`,
    expected: {
      url: '/net/data.json',
      status: 200,
      statusText: 'OK',
      ok: true,
      contentType: 'application/json',
      text: '{"path":"/net/data.json","query":""}',
      json: { path: '/net/data.json', query: '' },
    },
  },
  {
    name: 'params passed as an object reach the server and the url',
    source: `await page.goto(ORIGIN + '/net/blank.html')
    const response = await page.request.get(ORIGIN + '/net/data.json', {
      params: { p1: 'v1', 'парам2': 'знач2' },
    })
    return { url: path(response.url()), json: await response.json() }`,
    expected: {
      url: '/net/data.json?p1=v1&%D0%BF%D0%B0%D1%80%D0%B0%D0%BC2=%D0%B7%D0%BD%D0%B0%D1%872',
      json: {
        path: '/net/data.json',
        query:
          'p1=v1&%D0%BF%D0%B0%D1%80%D0%B0%D0%BC2=%D0%B7%D0%BD%D0%B0%D1%872',
      },
    },
  },
  {
    name: "a disposed response's body is gone; page.request is context.request",
    source: `await page.goto(ORIGIN + '/net/blank.html')
    const response = await page.request.get(ORIGIN + '/net/data.json')
    await response.dispose()
    return {
      same: page.request === context.request,
      body: await settle(response.body()),
    }`,
    expected: { same: true, body: { error: 'Error' } },
  },
]

test('page.request.get matches Playwright', {
  skip,
  timeout: 240_000,
}, async (t) => {
  await compareNetwork(t, cases)
})
