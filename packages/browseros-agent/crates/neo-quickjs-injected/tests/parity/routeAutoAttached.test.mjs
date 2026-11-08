// Routes over an out-of-process iframe that only the run's init scripts attached: Chromium
// attaches it held at its start (neo-runtime `init_scripts.rs`), and the tab's routes must reach
// its requests from then on, whether or not a call ever reaches the frame, through the pinned
// Playwright and through Neo (network.mjs). Playwright's `FrameSession._initialize` gives every
// frame session the page's interception before it runs.
//
// From the batch-4 delta review F1 (`review-delta-3d7f20365/route-auto-attached-child.test.mjs`):
// the child loads and runs its init script, then fetches only when told through `postMessage`,
// so no frame method ever reaches it unless a case asks for one.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, network.mjs).

import { test } from 'node:test'
import { skip } from './harness.mjs'
import { compareNetwork } from './network.mjs'

/** Creates the child on another site, waits for its ready message, then (after `between`)
 * tells it to fetch and returns what it fetched, with the route handler's count. */
const child = (between = '') => `
await page.evaluate((url) => {
  addEventListener('message', (event) => {
    if (event.data?.kind === 'ready') window.ready = true
    if (event.data?.kind === 'result') window.result = event.data
  })
  const frame = document.createElement('iframe')
  frame.id = frame.name = 'child'
  frame.src = url
  document.body.append(frame)
}, CROSS + '/net/ready-child.html')
await (await page.waitForFunction(() => window.ready, null, { timeout: 10_000 })).dispose()
${between}
await page.evaluate(() => document.querySelector('#child').contentWindow.postMessage('go', '*'))
const done = await page.waitForFunction(() => window.result, null, { timeout: 10_000 })
const result = await done.jsonValue()
await done.dispose()
return { result, hits }`

const routed = {
  result: { kind: 'result', origin: 'routed', seed: 13 },
  hits: 1,
}

const cases = [
  {
    name: 'a context route reaches an init-script iframe no call reached',
    source: `await page.goto(ORIGIN + '/net/blank.html')
await context.exposeFunction('unusedBinding', () => 1)
await context.addInitScript(() => { window.seed = 13 })
let hits = 0
await context.route('**/net/startup.json', async (route) => {
  hits++
  await route.fulfill({ json: { origin: 'routed' } })
})
${child()}`,
    expected: routed,
  },
  {
    name: 'once a call reaches that iframe too, its requests are routed once',
    source: `await page.goto(ORIGIN + '/net/blank.html')
await context.exposeFunction('unusedBinding', () => 1)
await context.addInitScript(() => { window.seed = 13 })
let hits = 0
await context.route('**/net/startup.json', async (route) => {
  hits++
  await route.fulfill({ json: { origin: 'routed' } })
})
${child("await page.frame('child').waitForSelector('#old', { timeout: 10_000 })")}`,
    expected: routed,
  },
  {
    name: 'a page route added after the init-script iframe started reaches it',
    source: `await page.goto(ORIGIN + '/net/blank.html')
await context.addInitScript(() => { window.seed = 13 })
let hits = 0
${child(`await page.route('**/net/startup.json', async (route) => {
  hits++
  await route.fulfill({ json: { origin: 'routed' } })
})`)}`,
    expected: routed,
  },
]

test('routes reach an init-script out-of-process iframe as in Playwright', {
  skip,
  timeout: 240_000,
}, async (t) => {
  await compareNetwork(t, cases)
})
