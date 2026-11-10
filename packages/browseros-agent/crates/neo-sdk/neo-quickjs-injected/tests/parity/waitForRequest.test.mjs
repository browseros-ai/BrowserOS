// page.waitForRequest through the pinned Playwright and through Neo (network.mjs): a glob string,
// a RegExp and a predicate (sync and async) each resolve with the first matching request the page
// makes after the call, as the same object its `request` event hands out; the timeout rejects
// with a TimeoutError; a page that closes first rejects the wait.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, network.mjs).

import { test } from 'node:test'
import { skip } from './harness.mjs'
import { compareNetwork } from './network.mjs'

const cases = [
  {
    // tests/page/page-wait-for-request.spec.ts "WA-121 should work", "WA-127 should work with url
    // match", "WA-128 should work with url match regular expression from a different context".
    name: 'a glob and a RegExp resolve with the first matching request',
    source: `await page.goto(ORIGIN + '/net/blank.html')
    const fetchAll = () => page.evaluate(() => {
      for (const url of ['/net/digits/1.json', '/net/digits/2.json', '/net/digits/3.json'])
        fetch(url).then((r) => r.text())
    })
    const [byGlob] = await Promise.all([page.waitForRequest(ORIGIN + '/net/digits/2.json'), fetchAll()])
    const seen = []
    page.on('request', (request) => seen.push(request))
    const [byRegExp] = await Promise.all([page.waitForRequest(/digits\\/\\d\\.json$/), fetchAll()])
    const [relativeGlob] = await Promise.all([page.waitForRequest('**/digits/3.json'), fetchAll()])
    return {
      byGlob: path(byGlob.url()),
      byRegExp: path(byRegExp.url()),
      relativeGlob: path(relativeGlob.url()),
      sameObject: seen.includes(byRegExp),
      method: byGlob.method(),
    }`,
    expected: {
      byGlob: '/net/digits/2.json',
      byRegExp: '/net/digits/1.json',
      relativeGlob: '/net/digits/3.json',
      sameObject: true,
      method: 'GET',
    },
  },
  {
    // page-wait-for-request.spec.ts "WA-122 should work with predicate".
    name: 'a predicate, sync or async, picks the request it accepts',
    source: `await page.goto(ORIGIN + '/net/blank.html')
    const fetchAll = () => page.evaluate(() => {
      fetch('/net/digits/1.json').then((r) => r.text())
      fetch('/net/echo', { method: 'POST', body: 'hello' }).then((r) => r.text())
    })
    const [sync] = await Promise.all([
      page.waitForRequest((request) => request.method() === 'POST'),
      fetchAll(),
    ])
    const [later] = await Promise.all([
      page.waitForRequest(async (request) => {
        await new Promise((resolve) => setTimeout(resolve, 10))
        return request.url().endsWith('/net/digits/1.json')
      }),
      fetchAll(),
    ])
    return { sync: [path(sync.url()), sync.postData()], later: path(later.url()) }`,
    expected: { sync: ['/net/echo', 'hello'], later: '/net/digits/1.json' },
  },
  {
    // page-wait-for-request.spec.ts "WA-123 should respect timeout"; a wait whose page closes
    // rejects (client/page.ts `_waitForEvent` `rejectOnEvent(this, Events.Page.Close, …)`).
    name: 'the timeout and a closing page reject the wait',
    source: `await page.goto(ORIGIN + '/net/blank.html')
    const timedOut = await settle(page.waitForRequest('**/never', { timeout: 100 }))
    const pending = settle(page.waitForRequest('**/never'))
    await page.close()
    return { timedOut, closed: await pending }`,
    // Playwright's TargetClosedError is named `Error`.
    expected: {
      timedOut: { error: 'TimeoutError' },
      closed: { error: 'Error' },
    },
  },
]

test('waitForRequest matches Playwright', {
  skip,
  timeout: 240_000,
}, async (t) => {
  await compareNetwork(t, cases)
})
