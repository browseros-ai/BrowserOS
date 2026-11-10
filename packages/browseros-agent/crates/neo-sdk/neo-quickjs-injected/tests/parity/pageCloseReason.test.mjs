// page.close({ reason }) through the pinned Playwright and through Neo, on fresh
// tests/pages/close-reason.html tabs (events.mjs): a wait still pending and a click still in
// flight on the page when it closes reject with the reason close() was given, while a call made
// after the close keeps the default closed message (client/page.ts `_closeErrorWithReason` for
// waiters; Playwright's server rejects calls in flight with the page's close reason). The script
// closes a page it opened, never the run's own tab, which events.mjs closes after the run.
// Rejections compare by `errorCategory` and by whether the message names the reason, never by
// wording.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, events.mjs).

import { test } from 'node:test'
import { compareEvents, fixture } from './events.mjs'
import { skip } from './harness.mjs'

const cases = [
  {
    // tests/library/page-close.spec.ts "PA-041 addLocatorHandler should throw when page closes"
    // (a click in flight rejects with 'custom reason'); tests/page/expect-misc.spec.ts "EX-243
    // should report page close reason when page closes during expect";
    // docs/src/api/class-page.md `Page.close.reason`.
    name: 'a pending wait and a click in flight reject with the reason; later calls do not',
    path: '/close-reason.html',
    source: `const other = await page.context().newPage()
    await other.goto(page.url())
    const reason = 'page bye'
    const outcome = (promise) => promise.then(
      () => 'resolved',
      (error) => ({ error: errorCategory(error), hasReason: error.message.includes(reason) }),
    )
    const waiting = outcome(other.waitForEvent('console'))
    const clicking = outcome(other.locator('#hidden').click())
    // Answered after the click has started: each engine takes a page's calls in order.
    const hidden = await other.isHidden('#hidden')
    const closed = await other.close({ reason })
    return {
      hidden,
      closed: [closed === undefined, other.isClosed()],
      waiting: await waiting,
      clicking: await clicking,
      later: [await outcome(other.title()), await outcome(other.locator('#count').click())],
    }`,
    // Playwright's TargetClosedError keeps the name Error.
    expected: {
      hidden: true,
      closed: [true, true],
      waiting: { error: 'Error', hasReason: true },
      clicking: { error: 'Error', hasReason: true },
      later: [
        { error: 'Error', hasReason: false },
        { error: 'Error', hasReason: false },
      ],
    },
    // The run's own tab was never clicked.
    probe: 'window.__clicks',
    effect: 0,
  },
]

test('page.close({ reason }) rejects the same calls the same way through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareEvents(t, {
    pages: { '/close-reason.html': fixture('close-reason.html') },
    cases,
  })
})
