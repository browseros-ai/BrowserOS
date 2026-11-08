// page.unroute and unrouteAll: removed handlers stop answering, and unrouteAll's behaviors wait
// for running handlers or ignore their errors, through the pinned Playwright and through Neo
// (network.mjs). Cases from the pinned tests (26137face):
//
// - tests/page/page-route.spec.ts RO-002 "should unroute", with tests/library/unroute-behavior.spec.ts
//   RO-193 "page.unrouteAll removes all routes": by handler, by URL, then all.
// - RO-194 "page.unrouteAll should wait for pending handlers to complete".
// - RO-189 "context.unrouteAll should not wait for pending handlers to complete if behavior is
//   ignoreErrors": the running handler's error is swallowed.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, network.mjs).

import { test } from 'node:test'
import { skip } from './harness.mjs'
import { compareNetwork } from './network.mjs'

const cases = [
  {
    name: 'unroute removes by handler, then by URL, and unrouteAll removes the rest',
    source: `let calls = []
    const add = (url, n) => page.route(url, (route) => {
      calls.push(n)
      return route.fallback()
    })
    await add('**/net/*', 1)
    await add('**/net/blank.html', 2)
    await add('**/net/blank.html', 3)
    const fourth = (route) => {
      calls.push(4)
      return route.fallback()
    }
    await page.route(/blank\\.html/, fourth)
    const visit = async () => {
      calls = []
      const response = await page.goto(ORIGIN + '/net/blank.html')
      return [calls, response.ok()]
    }
    const all = await visit()
    await page.unroute(/blank\\.html/, fourth)
    const byHandler = await visit()
    await page.unroute('**/net/blank.html')
    const byUrl = await visit()
    await page.unrouteAll()
    const none = await visit()
    return { all, byHandler, byUrl, none }`,
    expected: {
      all: [[4, 3, 2, 1], true],
      byHandler: [[3, 2, 1], true],
      byUrl: [[1], true],
      none: [[], true],
    },
  },
  {
    name: "unrouteAll({ behavior: 'wait' }) waits for the running handler",
    source: `let older = false
    let entered
    let release
    await page.route('**/net/blank.html', async (route) => {
      older = true
      await route.abort()
    })
    const inside = new Promise((resolve) => (entered = resolve))
    const barrier = new Promise((resolve) => (release = resolve))
    await page.route('**/net/blank.html', async (route) => {
      entered()
      await barrier
      await route.fallback()
    })
    const navigation = page.goto(ORIGIN + '/net/blank.html').then((response) => response.ok(), errorCategory)
    await inside
    const unrouting = page.unrouteAll({ behavior: 'wait' }).then(() => 'resolved')
    const early = await Promise.race([unrouting, new Promise((resolve) => setTimeout(() => resolve('pending'), 500))])
    release()
    return { early, late: await unrouting, navigation: await navigation, older }`,
    expected: {
      early: 'pending',
      late: 'resolved',
      navigation: true,
      older: false,
    },
  },
  {
    name: "context.unrouteAll({ behavior: 'ignoreErrors' }) does not wait, and swallows the handler's error",
    source: `let older = false
    let entered
    let release
    let threw
    await context.route(/\\/net\\/blank\\.html/, async (route) => {
      older = true
      await route.abort()
    })
    const inside = new Promise((resolve) => (entered = resolve))
    const barrier = new Promise((resolve) => (release = resolve))
    const thrown = new Promise((resolve) => (threw = resolve))
    await context.route(/\\/net\\/blank\\.html/, async () => {
      entered()
      await barrier
      threw()
      throw new Error('Handler error')
    })
    const navigation = page.goto(ORIGIN + '/net/blank.html', { timeout: 5000 }).then((response) => response.ok(), errorCategory)
    await inside
    const early = await Promise.race([
      context.unrouteAll({ behavior: 'ignoreErrors' }).then(() => 'resolved'),
      new Promise((resolve) => setTimeout(() => resolve('pending'), 2000)),
    ])
    release()
    await thrown
    await new Promise((resolve) => setTimeout(resolve, 300))
    return { early, navigation: await navigation, older }`,
    expected: { early: 'resolved', navigation: true, older: false },
  },
]

test('page.unroute and unrouteAll match Playwright', {
  skip,
  timeout: 240_000,
}, async (t) => {
  await compareNetwork(t, cases)
})
