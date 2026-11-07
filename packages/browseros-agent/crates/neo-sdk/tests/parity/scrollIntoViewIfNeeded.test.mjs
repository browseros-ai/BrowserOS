// Locator.scrollIntoViewIfNeeded waits for the element, then for it to be stable, and scrolls it
// into view with Chromium's DOM.scrollIntoViewIfNeeded, as Playwright's
// `ElementHandle._waitAndScrollIntoViewIfNeeded` does: an element below or right of the fold
// comes into view, one in view stays put, a visibility:hidden or zero-height element scrolls, a
// display:none element is retried until it shows or the timeout passes, and a strict locator, a
// missing element at its timeout and an invalid selector reject by kind. Where the element ends
// up is read with Locator.boundingBox, so both engines' layout is compared, not only the call.
//
// From Playwright's specs at the pin (tests/page/elementhandle-scroll-into-view.spec.ts at
// 26137face): 'EH-093 should work @smoke', 'EH-095 should wait for display:none to become
// visible', 'EH-097 should work for visibility:hidden element', 'EH-098 should work for
// zero-sized element' and 'EH-101 should timeout waiting for visible'.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

test('scrollIntoViewIfNeeded brings the element into view as Playwright does', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/scroll-into-view.html', import.meta.url),
      'utf8',
    ),
    source: `
      const box = async (selector) => {
        const b = await page.locator(selector).boundingBox()
        return b && [b.x, b.y, b.width, b.height].map(Math.round)
      }
      const scroll = async (selector, options) =>
        (await page.locator(selector).scrollIntoViewIfNeeded(options)) === undefined
      const steps = { before: await box('#far') }
      steps.far = [await scroll('#far'), await box('#far')]
      steps.again = [await scroll('#far', { timeout: 5000 }), await box('#far')]
      steps.visibilityHidden = [await scroll('#vis-hidden'), await box('#vis-hidden')]
      steps.zero = [await scroll('#zero'), await box('#zero')]
      steps.wide = [await scroll('#wide', { timeout: 5000 }), await box('#wide')]
      await page.click('#reveal')
      steps.later = [await scroll('#later'), await box('#later')]
      const failures = {}
      for (const [name, act] of [
        ['notVisible', () => page.locator('#none').scrollIntoViewIfNeeded({ timeout: 1000 })],
        ['strict', () => page.locator('.dup').scrollIntoViewIfNeeded()],
        ['missing', () => page.locator('#missing').scrollIntoViewIfNeeded({ timeout: 500 })],
        ['invalid', () => page.locator('[').scrollIntoViewIfNeeded()],
      ]) {
        try { await act(); failures[name] = 'resolved' } catch (error) { failures[name] = errorCategory(error) }
      }
      return { steps, failures }
    `,
    probe: 'window.__facts()',
    effects: ({ playwright, neo }) => {
      assert.deepEqual(neo, playwright, 'browser effect')
      assert.ok(playwright.scrollY > 0, 'the page scrolled down to #later')
    },
  })
  const { steps, failures } = outcome.value
  assert.deepEqual(failures, {
    notVisible: 'TimeoutError',
    strict: 'Error:strict',
    missing: 'TimeoutError',
    invalid: 'Error',
  })
  for (const name of [
    'far',
    'again',
    'visibilityHidden',
    'zero',
    'wide',
    'later',
  ])
    assert.equal(steps[name][0], true, `${name} resolves to undefined`)
  assert.ok(steps.before[1] > 1000, '#far starts below the fold')
  assert.ok(
    steps.far[1][1] >= 0 && steps.far[1][1] < steps.before[1],
    '#far scrolled up',
  )
  assert.deepEqual(steps.again[1], steps.far[1], 'an element in view stays put')
  assert.ok(
    steps.wide[1][0] >= 0 && steps.wide[1][0] < 2400,
    '#wide scrolled left',
  )
})
