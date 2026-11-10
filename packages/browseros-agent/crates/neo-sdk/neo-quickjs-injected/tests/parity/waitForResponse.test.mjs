// page.waitForResponse through the pinned Playwright and through Neo (network.mjs): a glob and a
// predicate (sync and async) resolve with the first matching response after the call, whose
// status, body and request are those of that response; the timeout rejects with a TimeoutError.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, network.mjs).

import { test } from 'node:test'
import { skip } from './harness.mjs'
import { compareNetwork } from './network.mjs'

const cases = [
  {
    // tests/page/page-wait-for-response.spec.ts "WA-129 should work".
    name: 'a glob resolves with the matching response, its body and request',
    source: `await page.goto(ORIGIN + '/net/blank.html')
    const [response] = await Promise.all([
      page.waitForResponse('**/net/digits/2.json'),
      // Each body is read: Chromium finishes loading a fetch only once its body is consumed.
      page.evaluate(() => {
        for (const url of ['/net/digits/1.json', '/net/digits/2.json?x=1', '/net/digits/2.json'])
          fetch(url).then((r) => r.text())
      }),
    ])
    return {
      url: path(response.url()),
      status: response.status(),
      json: await response.json(),
      request: path(response.request().url()),
      finished: await response.finished(),
    }`,
    expected: {
      url: '/net/digits/2.json',
      status: 200,
      json: { path: '/net/digits/2.json', query: '' },
      request: '/net/digits/2.json',
      finished: null,
    },
  },
  {
    // page-wait-for-response.spec.ts "WA-133 should work with predicate", "WA-134 should work with
    // async predicate".
    name: 'a predicate, sync or async, picks the response it accepts',
    source: `await page.goto(ORIGIN + '/net/blank.html')
    const fetchAll = () => page.evaluate(() => {
      for (const url of ['/net/404', '/net/digits/1.json']) fetch(url).then((r) => r.text())
    })
    const [missing] = await Promise.all([
      page.waitForResponse((response) => response.status() === 404),
      fetchAll(),
    ])
    const [json] = await Promise.all([
      page.waitForResponse(async (response) => (await response.text()).includes('digits')),
      fetchAll(),
    ])
    return { missing: [path(missing.url()), missing.ok(), await missing.text()], json: path(json.url()) }`,
    expected: {
      missing: ['/net/404', false, 'missing'],
      json: '/net/digits/1.json',
    },
  },
  {
    // page-wait-for-response.spec.ts "WA-130 should respect timeout".
    name: 'the timeout rejects with a TimeoutError',
    source: `await page.goto(ORIGIN + '/net/blank.html')
    return await settle(page.waitForResponse(/never/, { timeout: 100 }))`,
    expected: { error: 'TimeoutError' },
  },
]

test('waitForResponse matches Playwright', {
  skip,
  timeout: 240_000,
}, async (t) => {
  await compareNetwork(t, cases)
})
