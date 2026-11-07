// Binding calls a page makes during an awaited page call reach the script before that call's
// reply, and in the order the page made them, through the Playwright reference and through Neo.
// Playwright's server dispatches each `bindingCall` before the reply of the call it happened in
// (`server/page.ts` `PageBinding.dispatch`, events in order on one connection). Neo reads each
// binding call on a task of its own. Its session reader is one of the event hub's watchers, so a
// reply waits for the calls read before it, and the calls go in order (`neo-runtime`
// `bindings.rs`).
//
// `NEO_TEST_BINDING_READ_DELAYS_MS`, a debug-build-only hook in `bindings.rs`, holds Neo's
// reading of the 1st, 2nd and 3rd call for 150, 100 and 50 ms. Without the barrier the reply
// overtakes all three; without the order they arrive 3, 2, 1. Every wait is within the hub's
// 250 ms barrier bound.
//
// Regression for #3198's clock failure: runFor's timers called a binding whose call reached the
// script after runFor's reply.
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

test('binding calls during a page call reach the script before its reply, in order', {
  skip,
  timeout: 180_000,
}, async (t) => {
  process.env.NEO_TEST_BINDING_READ_DELAYS_MS = '150,100,50'
  try {
    await compareMethod(t, {
      html: '<title>binding order</title><p>fixture</p>',
      isolate: true,
      source: `
        const calls = []
        await page.exposeFunction('stub', (value) => { calls.push(value) })
        await page.evaluate(() => { window.stub(1); window.stub(2); window.stub(3) })
        const during = [...calls]
        await page.evaluate(() => { window.stub(4) })
        return { during, after: [...calls] }
      `,
      probe: 'document.title',
    })
  } finally {
    delete process.env.NEO_TEST_BINDING_READ_DELAYS_MS
  }
})
