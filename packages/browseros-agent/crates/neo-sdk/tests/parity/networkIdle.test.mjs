// `networkidle` through the pinned Playwright and through Neo (network.mjs): goto with
// `waitUntil: 'networkidle'` resolves only after a fetch the page starts after its load has
// finished (`load` resolves before it), a child frame's late fetch holds the parent too, an open
// EventSource holds nothing, and waitForLoadState('networkidle') waits the same way. Each page
// retitles itself `done` when its late fetch finished; the title read right after the wait tells
// whether the wait outlived the fetch. The same holds for reload, goBack, goForward, waitForURL
// and waitForNavigation with `waitUntil: 'networkidle'`.
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
    // page-network-idle.spec.ts "NE-073 should wait for networkidle in waitForNavigation", and
    // the same option on reload, goBack, goForward and waitForURL: each resolves only after the
    // late fetch of the document it reached finished.
    name: 'reload, goBack, goForward, waitForURL and waitForNavigation wait for networkidle',
    source: `const late = ORIGIN + '/net/late.html'
    const titles = {}
    await page.goto(late, { waitUntil: 'networkidle' })
    await page.reload({ waitUntil: 'networkidle' })
    titles.reload = await page.title()
    await page.goto(ORIGIN + '/net/blank.html')
    await page.goBack({ waitUntil: 'networkidle' })
    titles.goBack = await page.title()
    await page.goForward({ waitUntil: 'load' })
    await page.goBack({ waitUntil: 'load' })
    titles.afterLoadOnly = await page.title()
    await page.goForward({ waitUntil: 'networkidle' })
    titles.goForward = await page.title()
    await Promise.all([
      page.waitForNavigation({ waitUntil: 'networkidle' }),
      page.evaluate((url) => { location.href = url }, late),
    ])
    titles.waitForNavigation = await page.title()
    await page.goto(ORIGIN + '/net/blank.html')
    await Promise.all([
      page.waitForURL('**/net/late.html', { waitUntil: 'networkidle' }),
      page.evaluate((url) => { location.href = url }, late),
    ])
    titles.waitForURL = await page.title()
    return titles`,
    expected: {
      reload: 'done',
      goBack: 'done',
      afterLoadOnly: 'late',
      goForward: 'blank',
      waitForNavigation: 'done',
      waitForURL: 'done',
    },
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
  // History steps run as in history.mjs: Playwright's Chromium never restores a document from
  // the back-forward cache (neo-runtime history_e2e.rs covers Neo's restore).
  await compareNetwork(t, cases, { args: ['--disable-back-forward-cache'] })
})
