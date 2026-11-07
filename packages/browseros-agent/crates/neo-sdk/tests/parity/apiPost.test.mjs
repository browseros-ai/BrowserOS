// APIRequestContext.post: the body encodings Playwright's server sends, seen by the server's echo
// (`/net/headers`: method, headers, body), through the pinned Playwright and through Neo
// (network.mjs). Cases from the pinned tests (26137face):
//
// - tests/library/browsercontext-fetch.spec.ts FE-157 "should encode to application/json by
//   default": an object `data` goes as JSON.
// - FE-155 "should support application/x-www-form-urlencoded": `form` goes url-encoded.
// - FE-158 "should support multipart/form-data": a field and a `{ name, mimeType, buffer }` file;
//   the random boundary is compared as `B`.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, network.mjs).

import { test } from 'node:test'
import { skip } from './harness.mjs'
import { compareNetwork } from './network.mjs'

const ECHOED = `const echoed = await response.json()
    const contentType = echoed.headers['content-type'] ?? null
    const boundary = contentType?.split('boundary=')[1]
    const normalize = (text) => (boundary ? text.split(boundary).join('B') : text)
    return {
      method: echoed.method,
      contentType: contentType && normalize(contentType),
      length: echoed.headers['content-length'] ?? null,
      body: normalize(echoed.body),
    }`

const cases = [
  {
    name: 'an object data goes as JSON',
    source: `await page.goto(ORIGIN + '/net/blank.html')
    const response = await page.request.post(ORIGIN + '/net/headers', { data: { a: 1, b: 'two' } })
    ${ECHOED}`,
    expected: {
      method: 'POST',
      contentType: 'application/json',
      length: '17',
      body: '{"a":1,"b":"two"}',
    },
  },
  {
    name: 'form goes url-encoded',
    source: `await page.goto(ORIGIN + '/net/blank.html')
    const response = await page.request.post(ORIGIN + '/net/headers', {
      form: { first: 'John', last: 'Doe & Co', num: 1 },
    })
    ${ECHOED}`,
    expected: {
      method: 'POST',
      contentType: 'application/x-www-form-urlencoded',
      length: '32',
      body: 'first=John&last=Doe+%26+Co&num=1',
    },
  },
  {
    name: 'multipart sends a field and a file',
    source: `await page.goto(ORIGIN + '/net/blank.html')
    // Playwright takes a Node Buffer; Neo's realm has none, and takes any Uint8Array.
    const buffer = typeof Buffer === 'function' ? Buffer.from('var x = 10;') : Uint8Array.from('var x = 10;', (char) => char.charCodeAt(0))
    const response = await page.request.post(ORIGIN + '/net/headers', {
      multipart: { first: 'John', file: { name: 'f.js', mimeType: 'text/javascript', buffer } },
    })
    ${ECHOED}`,
    expected: {
      method: 'POST',
      contentType: 'multipart/form-data; boundary=B',
      length: '290',
      body:
        '--B\r\ncontent-disposition: form-data; name="first"\r\n\r\nJohn\r\n' +
        '--B\r\ncontent-disposition: form-data; name="file"; filename="f.js"\r\ncontent-type: text/javascript\r\n\r\nvar x = 10;\r\n' +
        '--B--\r\n',
    },
  },
]

test('APIRequestContext.post matches Playwright', {
  skip,
  timeout: 240_000,
}, async (t) => {
  await compareNetwork(t, cases)
})
