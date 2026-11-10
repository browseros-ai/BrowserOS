// page.setViewportSize through Neo and through Playwright on one BrowserOS neo, each case on a
// fresh tab of tests/pages/emulation.html: what the script returns, and what the page itself
// reports of its viewport, screen and width media query once the call resolved.
//
// Playwright cases (pin 26137face): `tests/library/browsercontext-viewport.spec.ts`
// "VP-002 should set the proper viewport size" (the size takes effect), and its validator
// rejecting a fractional size before any effect.
//
// Deviation, asserted: Neo's override ends with the run (Chromium drops it when the run's
// session detaches); Playwright's lasts while it is connected.

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { skip } from './harness.mjs'
import { fixturePage, outcomeValue, withPlatform } from './platform.mjs'

const pages = { '/emulation.html': fixturePage('emulation.html') }

const cases = [
  {
    name: 'sets the viewport and screen',
    source: `const returned = await page.setViewportSize({ width: 500, height: 400 })
const s = await settle((s) => s.innerWidth === 500 && s.innerHeight === 400)
return { returned: returned === undefined, size: [s.innerWidth, s.innerHeight], screen: s.screen,
  narrow: s.media['(max-width: 600px)'], viewport: page.viewportSize() }`,
    expected: {
      returned: true,
      size: [500, 400],
      screen: [500, 400],
      narrow: true,
      viewport: { width: 500, height: 400 },
    },
  },
  {
    name: 'a second size replaces the first',
    source: `await page.setViewportSize({ width: 500, height: 400 })
await settle((s) => s.innerWidth === 500)
await page.setViewportSize({ width: 700, height: 300 })
const s = await settle((s) => s.innerWidth === 700 && s.innerHeight === 300)
return { size: [s.innerWidth, s.innerHeight], narrow: s.media['(max-width: 600px)'], viewport: page.viewportSize() }`,
    expected: {
      size: [700, 300],
      narrow: false,
      viewport: { width: 700, height: 300 },
    },
  },
  {
    name: 'a fractional size is refused before any effect',
    source: `const before = await state()
const error = await page.setViewportSize({ width: 300.5, height: 200 }).then(() => null, errorCategory)
const after = await state()
return { error, unchanged: before.innerWidth === after.innerWidth && before.innerHeight === after.innerHeight,
  viewport: page.viewportSize() }`,
    // Playwright's client records the size before the call, so viewportSize() answers it even
    // though the call failed; Neo's keeps that behavior.
    expected: {
      error: 'Error',
      unchanged: true,
      viewport: { width: 300.5, height: 200 },
    },
  },
]

test('setViewportSize: Neo matches Playwright', { skip }, async (t) => {
  await withPlatform(t, { pages }, async ({ playwright, neo, cdp }) => {
    for (const { name, source, expected } of cases) {
      const reference = outcomeValue(
        await playwright('/emulation.html', source),
      )
      const actual = await neo('/emulation.html', source)
      assert.deepEqual(reference, expected, `${name}: Playwright`)
      assert.deepEqual(outcomeValue(actual), reference, `${name}: Neo`)
    }
    // Deviation: the run's override is gone once the run ended.
    const after = await neo(
      '/emulation.html',
      `await page.setViewportSize({ width: 333, height: 222 })
return (await settle((s) => s.innerWidth === 333)).innerWidth`,
    )
    assert.equal(outcomeValue(after), 333)
    const { sessionId } = await cdp.send('Target.attachToTarget', {
      targetId: after.targetId,
      flatten: true,
    })
    const { result } = await cdp.send(
      'Runtime.evaluate',
      { expression: 'innerWidth', returnByValue: true },
      sessionId,
    )
    assert.notEqual(result.value, 333, 'the override ended with the run')
  })
})
