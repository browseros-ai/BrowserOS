// `networkidle` through the pinned Playwright and through Neo (network.mjs): goto with
// `waitUntil: 'networkidle'` resolves only after a fetch the page starts after its load has
// finished (`load` resolves before it), a child frame's late fetch holds the parent too, an open
// EventSource holds nothing, and waitForLoadState('networkidle') waits the same way. Each page
// retitles itself `done` when its late fetch finished; the title read right after the wait tells
// whether the wait outlived the fetch.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, network.mjs).

import { test } from 'node:test'
import { skip } from './harness.mjs'
import { compareNetwork } from './network.mjs'

const cases = [
  {
    // tests/page/page-network-idle.spec.ts "NE-071 should wait for networkidle to succeed
    // navigation".
    name: 'goto networkidle resolves only after a late fetch finished',
    source: `const load = await page.goto(ORIGIN + '/net/late.html', { waitUntil: 'load' })
    const atLoad = await page.title()
    const idle = await page.goto(ORIGIN + '/net/late.html', { waitUntil: 'networkidle' })
    return { atLoad, loadStatus: load.status(), atIdle: await page.title(), idleStatus: idle.status() }`,
    expected: {
      atLoad: 'late',
      loadStatus: 200,
      atIdle: 'done',
      idleStatus: 200,
    },
  },
  {
    // page-network-idle.spec.ts "NE-078 should wait for networkidle from the child frame",
    // "NE-073 should wait for networkidle in waitForNavigation" (as waitForLoadState).
    name: "a child frame's late fetch holds the parent's networkidle, in goto and waitForLoadState",
    source: `await page.goto(ORIGIN + '/net/late-frame.html', { waitUntil: 'networkidle' })
    const viaGoto = await page.title()
    await page.goto(ORIGIN + '/net/late.html', { waitUntil: 'load' })
    const beforeWait = await page.title()
    await page.waitForLoadState('networkidle')
    return { viaGoto, beforeWait, afterWait: await page.title() }`,
    expected: { viaGoto: 'done', beforeWait: 'late', afterWait: 'done' },
  },
  {
    // page-network-idle.spec.ts "NE-082 should not wait for an open EventSource connection".
    name: 'an open EventSource does not hold networkidle',
    source: `const response = await page.goto(ORIGIN + '/net/event-source.html', { waitUntil: 'networkidle', timeout: 10000 })
    return { status: response.status(), title: await page.title() }`,
    expected: { status: 200, title: 'events' },
  },
]

test('networkidle matches Playwright', {
  skip,
  timeout: 240_000,
}, async (t) => {
  await compareNetwork(t, cases)
})
