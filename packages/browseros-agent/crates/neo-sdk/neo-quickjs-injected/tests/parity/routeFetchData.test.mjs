// route.fetch's request data, typed as Playwright's client types it (`client/fetch.ts`
// `APIRequestContext._innerFetch`) and with the content type its server gives each kind
// (`server/fetch.ts` `serializePostData`): JSON for an object, number, boolean or null, or for a
// string under a JSON content type; `application/octet-stream` for other strings and bytes,
// including the request's own body; a content type the request or the script gave stays. Through
// the pinned Playwright and through Neo (network.mjs).
//
// From the batch-4 delta review F2 (`review-delta-3d7f20365/route-json-service.test.mjs`,
// `route-binary.test.mjs`): a service that requires JSON answers 415 to a body sent without its
// content type.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, network.mjs).

import { test } from 'node:test'
import { skip } from './harness.mjs'
import { compareNetwork } from './network.mjs'

const hex = (text) => Buffer.from(text).toString('hex')
/** What `/net/post-echo` answers for a POST of `type` whose body is `text`, or `bytes` (hex). */
const echo = (type, text) => ({ method: 'POST', type, hex: hex(text) })
const echoBytes = (type, bytes) => ({ method: 'POST', type, hex: bytes })
const JSON_TYPE = 'application/json'
const BYTES_TYPE = 'application/octet-stream'

const cases = [
  {
    name: 'JSON data reaches a service that requires its content type',
    source: `await page.goto(ORIGIN + '/net/blank.html')
const call = () => page.evaluate(async () => {
  const response = await fetch('/net/require-json')
  return { status: response.status, body: await response.json() }
})
await page.route('**/net/require-json', async (route) => {
  const response = await route.fetch({ method: 'POST', postData: { value: 0 } })
  await route.fulfill({ response })
})
const inferred = await call()
await page.unroute('**/net/require-json')
await page.route('**/net/require-json', async (route) => {
  const response = await route.fetch({
    method: 'POST',
    postData: { value: 0 },
    headers: { 'content-type': 'application/json' },
  })
  await route.fulfill({ response })
})
const explicit = await call()
return { inferred, explicit }`,
    expected: {
      inferred: { status: 200, body: { ok: true, value: { value: 0 } } },
      explicit: { status: 200, body: { ok: true, value: { value: 0 } } },
    },
  },
  {
    name: 'each kind of postData goes with its own body and content type',
    source: `await page.goto(ORIGIN + '/net/blank.html')
const bytes = (values) => (typeof Buffer === 'function' ? Buffer.from(values) : new Uint8Array(values))
const json = { 'content-type': 'application/json' }
const kinds = {
  object: [{ value: 0 }],
  false: [false],
  null: [null],
  zero: [0],
  text: ['text'],
  jsonText: ['{"a":1}'],
  bytes: [bytes([0, 255, 10])],
  textAsJson: ['plain', json],
  jsonTextAsJson: ['{"a":1}', json],
}
const fetched = {}
await page.route(/\\/net\\/post-echo/, async (route) => {
  const kind = /kind=([^&]+)/.exec(route.request().url())[1]
  const [postData, headers] = kinds[kind]
  const response = await route.fetch({ method: 'POST', postData, headers })
  fetched[kind] = await response.json()
  await route.fulfill({ response })
})
for (const kind of Object.keys(kinds))
  await page.evaluate((kind) => fetch('/net/post-echo?kind=' + kind).then((r) => r.json()), kind)
return fetched`,
    expected: {
      object: echo(JSON_TYPE, '{"value":0}'),
      false: echo(JSON_TYPE, 'false'),
      null: echo(JSON_TYPE, 'null'),
      zero: echo(JSON_TYPE, '0'),
      text: echo(BYTES_TYPE, 'text'),
      jsonText: echo(BYTES_TYPE, '{"a":1}'),
      bytes: echoBytes(BYTES_TYPE, '00ff0a'),
      textAsJson: echo(JSON_TYPE, '"plain"'),
      jsonTextAsJson: echo(JSON_TYPE, '{"a":1}'),
    },
  },
  {
    name: "with no postData, the request's own body goes, typed as bytes unless it has a type",
    source: `await page.goto(ORIGIN + '/net/blank.html')
const fetched = {}
await page.route(/\\/net\\/post-echo/, async (route) => {
  const kind = /kind=([^&]+)/.exec(route.request().url())[1]
  const response = await route.fetch()
  fetched[kind] = await response.json()
  await route.fulfill({ response })
})
await page.route(/kind=fallback/, (route) => route.fallback({ method: 'POST', postData: { a: 1 } }))
await page.evaluate(() => fetch('/net/post-echo?kind=fallback').then((r) => r.json()))
await page.evaluate(() => fetch('/net/post-echo?kind=bytes', { method: 'POST', body: new Uint8Array([1, 2, 3]) }).then((r) => r.json()))
await page.evaluate(() => fetch('/net/post-echo?kind=typed', { method: 'POST', body: 'x', headers: { 'content-type': 'text/plain' } }).then((r) => r.json()))
return fetched`,
    expected: {
      fallback: echo(BYTES_TYPE, '{"a":1}'),
      bytes: echoBytes(BYTES_TYPE, '010203'),
      typed: echo('text/plain', 'x'),
    },
  },
]

test('route.fetch types its postData as Playwright does', {
  skip,
  timeout: 240_000,
}, async (t) => {
  await compareNetwork(t, cases)
})
