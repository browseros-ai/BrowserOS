// setContent and networkidle: the written document has a lifecycle of its own, as Playwright's
// `Frame._onClearLifecycle` gives it right after `document.open()`, so `waitUntil: 'networkidle'`
// and a later `waitForLoadState('networkidle')` wait for the written document's requests, never
// for the old document's idleness. Through the pinned Playwright and through Neo (network.mjs).
//
// - The delta review's F3 (med-pw-int review-delta-995823580.md), its three cases ported to this
//   runner: setContent accepts `networkidle`; after the old document went idle, a later
//   `waitForLoadState('networkidle')` waits for the written document's fetch; and its control, a
//   request listener before the first write with no idle state before it.
// - tests/page/page-network-idle.spec.ts NE-074 "should wait for networkidle in setContent": the
//   written document's late fetch holds `networkidle`.
// - NE-077 "should wait for networkidle in setContent from the child frame": a frame the written
//   document creates holds it too.
// - A child frame's own setContent: the frame's lifecycle clears, and its late fetch holds it.
// - A fresh page's first setContent with `networkidle`: the call watches the network itself.
// - The #3216 re-check's out-of-process frame cases (warm and cold, no requests and a slow body):
//   a deliberate difference, Playwright resolves and Neo refuses before writing.
//
// Every fetch reads its body: Chromium reports a fetch whose body is never read as still loading
// (measured on BrowserOS neo: no `requestfinished` within 3 s, for plain, JSON and HTML bodies),
// so neither engine's `networkidle` would come until a garbage collection drops the body.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, network.mjs).

import { test } from 'node:test'
import { skip } from './harness.mjs'
import { compareNetwork } from './network.mjs'

// A fetch 300 ms after the written document runs, whose response takes 700 ms; the title says
// when it finished.
const LATE = `<title>late</title><script>
setTimeout(() => fetch('/net/slow?ms=700').then((r) => r.text()).then(() => { document.title = 'done' }), 300)
</script>`

// The written document's fetch, 900 ms, whose body is read; it says when it finished.
const SLOW_BODY =
  '<p id="new">New</p><script>window.done = false; fetch("/net/slow?ms=900").then((r) => r.text()).then(() => { window.done = true })</script>'

const cases = [
  {
    name: 'setContent waits for networkidle',
    source: `await page.goto(ORIGIN + '/net/blank.html')
    const result = await settle(page.setContent('<p id="new">new</p>', { waitUntil: 'networkidle', timeout: 2500 }))
    return { result, changed: await page.locator('#new').count() }`,
    expected: { result: 'undefined', changed: 1 },
  },
  {
    name: "after the old document went idle, waitForLoadState('networkidle') waits for the written one",
    source: `await page.goto(ORIGIN + '/net/blank.html')
    await page.waitForLoadState('networkidle', { timeout: 4000 })
    await page.setContent('<p id="new">new</p><script>window.done = false; fetch("/net/slow?ms=900").then((r) => r.text()).then(() => { window.done = true })</script>')
    const result = await settle(page.waitForLoadState('networkidle', { timeout: 3000 }))
    return {
      result,
      finished: await page.evaluate(() => window.done),
      changed: await page.locator('#new').count(),
    }`,
    expected: { result: 'undefined', finished: true, changed: 1 },
  },
  {
    name: 'control: a request listener before the first write, with no idle state before it',
    source: `page.on('request', () => {})
    const slow = JSON.stringify(ORIGIN + '/net/slow?ms=900')
    await page.setContent('<p id="new">new</p><script>window.done = false; fetch(' + slow + ').then((r) => r.text()).then(() => { window.done = true })</script>')
    const result = await settle(page.waitForLoadState('networkidle', { timeout: 3000 }))
    return { result, finished: await page.evaluate(() => window.done) }`,
    expected: { result: 'undefined', finished: true },
  },
  {
    // Nothing before the write watched the network: the call itself must, as Playwright's
    // server always does, or the written document's fetch would go unseen.
    name: "a fresh page's first setContent with networkidle waits for its document's fetch",
    source: `const slow = JSON.stringify(ORIGIN + '/net/slow?ms=900')
    const result = await settle(page.setContent('<script>window.done = false; fetch(' + slow + ').then((r) => r.text()).then(() => { window.done = true })</script>', { waitUntil: 'networkidle' }))
    return { result, finished: await page.evaluate(() => window.done) }`,
    expected: { result: 'undefined', finished: true },
  },
  {
    name: "waitUntil: 'networkidle' waits for the written document's late fetch",
    source: `await page.goto(ORIGIN + '/net/blank.html', { waitUntil: 'networkidle' })
    const result = await settle(page.setContent(${JSON.stringify(LATE)}, { waitUntil: 'networkidle' }))
    return { result, title: await page.title() }`,
    expected: { result: 'undefined', title: 'done' },
  },
  {
    name: 'a frame the written document creates holds its networkidle',
    source: `await page.goto(ORIGIN + '/net/blank.html', { waitUntil: 'networkidle' })
    const result = await settle(page.setContent('<title>parent</title><iframe src="/net/late-child.html"></iframe>', { waitUntil: 'networkidle' }))
    return { result, title: await page.title() }`,
    expected: { result: 'undefined', title: 'done' },
  },
  {
    name: "a child frame's setContent waits for its own late fetch",
    source: `await page.goto(ORIGIN + '/net/frame-top.html', { waitUntil: 'networkidle' })
    const child = page.frame({ name: 'child' })
    const result = await settle(child.setContent(${JSON.stringify(LATE)}, { waitUntil: 'networkidle' }))
    return { result, title: await child.title() }`,
    expected: { result: 'undefined', title: 'done' },
  },
  ...['warm', 'cold'].flatMap((warmth) =>
    [false, true].map((slow) => ({
      // The #3216 re-check (review-delta-995823580.md): Playwright waits for an out-of-process
      // frame's networkidle as for any frame's. Neo's idle tracker follows the tab's own session
      // only, so the wait could only time out: Neo refuses it before writing (Deviation).
      name: `an out-of-process frame's setContent with networkidle, ${warmth}, ${slow ? 'a slow body' : 'no requests'}`,
      source: `await page.goto(ORIGIN + '/net/oopif-top.html')
    const frame = page.frame('cross')
    await frame.waitForSelector('#fetch')
    ${warmth === 'warm' ? "await frame.waitForLoadState('load', { timeout: 3000 })" : ''}
    const result = await settle(frame.setContent(${JSON.stringify(slow ? SLOW_BODY : '<p id="new">New</p>')}, { waitUntil: 'networkidle', timeout: 2500 }))
    return {
      result,
      changed: await frame.locator('#new').count(),
      done: await frame.evaluate(() => window.done ?? null),
    }`,
      playwright: { result: 'undefined', changed: 1, done: slow ? true : null },
      neo: { result: { error: 'Error' }, changed: 0, done: null },
    })),
  ),
]

test('setContent and networkidle match Playwright', {
  skip,
  timeout: 240_000,
}, async (t) => {
  await compareNetwork(t, cases)
})
