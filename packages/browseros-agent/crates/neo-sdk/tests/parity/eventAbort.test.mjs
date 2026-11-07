// Playwright's `signal` option on the waits for A3's events, through the pinned Playwright and
// through Neo (the realm's `AbortController`), on fresh tests/pages/event-abort.html tabs
// (events.mjs): `waitForEvent('filechooser')` and `waitForEvent('download')` on the page and on
// the context reject with `AbortError` whose `cause` is the reason, whether the signal aborts
// mid-wait or before the call, and leave nothing listening: a later click on a file input opens
// the browser's own dialog (no interception is left), a later download reaches a new wait.
// Errors compare by `name` and whether `cause` is the reason, never by wording.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, events.mjs).

import { test } from 'node:test'
import { compareEvents, fixture } from './events.mjs'
import { skip } from './harness.mjs'

/** How a wait settled, against the reason its signal aborted with. */
const ABORTED = `const aborted = (promise, reason) => promise.then(
      () => 'resolved',
      (error) => ({ name: error.name, causeIsReason: error.cause === reason }),
    )
    `

const cases = [
  {
    // tests/page/page-filechooser.spec.ts "FC-015 should abort with signal".
    name: "page.waitForEvent('filechooser') aborted mid-wait rejects and leaves no interception",
    path: '/event-abort.html',
    source: `${ABORTED}const controller = new AbortController()
    const waiting = page.waitForEvent('filechooser', { signal: controller.signal })
    controller.abort('Aborted by user')
    const settled = await aborted(waiting, 'Aborted by user')
    await page.click('#single')
    await page.waitForFunction(() => window.__cancels > 0, null, { timeout: 5000 })
    return {
      settled,
      listeners: page.listenerCount('filechooser'),
      cancels: await page.evaluate(() => window.__cancels),
    }`,
    expected: {
      settled: { name: 'AbortError', causeIsReason: true },
      listeners: 0,
      cancels: 1,
    },
    probe: 'window.__cancels',
    effect: 1,
  },
  {
    // tests/page/page-click.spec.ts "CL-092 should abort via already-aborted signal", on the
    // context's download wait.
    name: "context.waitForEvent('download') with an aborted signal rejects at once and listens to nothing",
    path: '/event-abort.html',
    source: `${ABORTED}const controller = new AbortController()
    const reason = new Error('stop')
    controller.abort(reason)
    const settled = await aborted(page.context().waitForEvent('download', { signal: controller.signal }), reason)
    const listeners = page.context().listenerCount('download')
    const [download] = await Promise.all([page.waitForEvent('download'), page.click('#data')])
    return { settled, listeners, later: download.suggestedFilename() }`,
    expected: {
      settled: { name: 'AbortError', causeIsReason: true },
      listeners: 0,
      later: 'data.txt',
    },
  },
  {
    // page-click.spec.ts "CL-090 should abort via signal", on the page's download wait.
    name: "page.waitForEvent('download') aborted mid-wait rejects and stops listening",
    path: '/event-abort.html',
    source: `${ABORTED}const controller = new AbortController()
    const waiting = page.waitForEvent('download', { signal: controller.signal })
    await new Promise((resolve) => setTimeout(resolve, 100))
    controller.abort('later')
    return { settled: await aborted(waiting, 'later'), listeners: page.listenerCount('download') }`,
    expected: {
      settled: { name: 'AbortError', causeIsReason: true },
      listeners: 0,
    },
  },
]

test('the waits for downloads and file choosers honor signal the same way through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareEvents(t, {
    pages: { '/event-abort.html': fixture('event-abort.html') },
    cases,
  })
})
