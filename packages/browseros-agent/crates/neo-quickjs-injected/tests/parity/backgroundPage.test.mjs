// context.on('backgroundpage') through the pinned Playwright and through Neo, on a fresh
// tests/pages/opener.html tab (events.mjs): both take the listener, and neither ever emits it,
// across a popup opening and closing, as Playwright documents for the deprecated event
// (docs/src/api/class-browsercontext.md "event: BrowserContext.backgroundPage": not emitted;
// client/events.ts: "never emitted anymore"). The popup is closed before the case returns.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, events.mjs).

import { test } from 'node:test'
import { compareEvents, fixture } from './events.mjs'
import { skip } from './harness.mjs'

const cases = [
  {
    name: "'backgroundpage' takes a listener and never fires",
    path: '/opener.html',
    source: `const seen = []
    const listener = (backgroundPage) => seen.push(path(backgroundPage.url()))
    page.context().on('backgroundpage', listener)
    const taken = page.context().listenerCount('backgroundpage')
    const [popup] = await Promise.all([page.waitForEvent('popup'), page.click('#open')])
    await popup.close()
    page.context().off('backgroundpage', listener)
    return { taken, seen, left: page.context().listenerCount('backgroundpage') }`,
    expected: { taken: 1, seen: [], left: 0 },
    probe: 'window.__opened',
    effect: 1,
  },
]

test("'backgroundpage' is quiet through Neo and Playwright", {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareEvents(t, {
    pages: {
      '/opener.html': fixture('opener.html'),
      '/popup.html': fixture('popup.html'),
    },
    cases,
  })
})
