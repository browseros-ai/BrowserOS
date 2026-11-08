// A route whose handlers all pass on a request: the SDK continues it on its own (Playwright's
// `BrowserContext._onRoute`: `await route._innerContinue(true /* isFallback */).catch(() => {});`),
// and that continuation must reach the browser even when the script has as many calls in flight
// as Neo allows (64, neo-runtime `dispatch.rs` `MAX_CALLS_IN_FLIGHT`): the script made 64 calls,
// not 65, and the request it waits on is the one the continuation lets go. Through the pinned
// Playwright and through Neo (network.mjs).
//
// From the batch-4 delta review F3 (`review-delta-3d7f20365/route-auto-continue-cap.test.mjs`,
// `route-glob-cap-control.test.mjs`). The 63 waits here end only when the script says so, once
// the fetch settled, so the cases force their timing instead of racing timers; a fetch the
// continuation never let go reads `unsettled` after 10 s.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, network.mjs).

import { test } from 'node:test'
import { skip } from './harness.mjs'
import { compareNetwork } from './network.mjs'

/** With `held` waits in flight, fetches through a route registered by `route` whose handler
 * never takes the request; the waits then end. */
const capped = (route, held) => `await page.goto(ORIGIN + '/net/blank.html')
let calls = 0
await ${route}
const waiting = Array.from({ length: ${held} }, () =>
  page.waitForFunction(() => window.release === true, null, { timeout: 60_000 }))
const result = await Promise.race([
  page.evaluate(async () => await (await fetch('/net/data.json')).json()).then(
    (value) => ({ value }),
    (error) => ({ error: errorCategory(error) }),
  ),
  new Promise((resolve) => setTimeout(() => resolve({ unsettled: true }), 10_000)),
])
await page.evaluate(() => { window.release = true })
await Promise.all(waiting.map((wait) => wait.then((handle) => handle.dispose())))
return { result, calls }`

const fetched = {
  result: { value: { path: '/net/data.json', query: '' } },
  calls: 0,
}

const cases = [
  {
    name: "a page predicate route's own continuation goes at the 64th call",
    source: capped('page.route(() => false, () => { calls++ })', 63),
    expected: fetched,
  },
  {
    name: "a context predicate route's own continuation goes at the 64th call",
    source: capped('context.route(() => false, () => { calls++ })', 63),
    expected: fetched,
  },
  {
    name: 'below the cap, the same route goes on (control)',
    source: capped('page.route(() => false, () => { calls++ })', 62),
    expected: fetched,
  },
  {
    name: 'a glob no request matches never pauses one (control)',
    source: capped("page.route('**/not-this-endpoint', () => { calls++ })", 63),
    expected: fetched,
  },
]

test("an unmatched route's own continuation passes the call cap", {
  skip,
  timeout: 240_000,
}, async (t) => {
  await compareNetwork(t, cases)
})
