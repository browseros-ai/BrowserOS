// Locator.hideHighlight and Page.hideHighlight take highlights down as Playwright's
// HighlightController does: hiding a locator's highlight empties its document's overlay but
// leaves the overlay in place (Playwright sets that document's highlights to none), hiding one
// that was never shown changes nothing, and Page.hideHighlight removes the overlay of every
// document, a child frame's included, even once its highlights were hidden. A run that ends
// with no overlay left is the same through both engines.
//
// From Playwright's specs at the pin (tests/page/locator-highlight.spec.ts at 26137face): 'LO-114
// should highlight locator', with the highlight then hidden through each receiver.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

test('hideHighlight empties or removes the overlay as Playwright does', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/highlight.html', import.meta.url),
      'utf8',
    ),
    source: `
      const steps = []
      const step = async (name, act) => steps.push([name, (await act()) === undefined])
      await page.locator('#target').highlight()
      await step('locator', () => page.locator('#target').hideHighlight())
      await step('neverShown', () => page.locator('#other').hideHighlight())
      await page.frameLocator('#child').locator('#inner').highlight()
      await step('frame', () => page.frameLocator('#child').locator('#inner').hideHighlight())
      await step('page', () => page.hideHighlight())
      await step('pageAgain', () => page.hideHighlight())
      await page.locator('.dup').highlight()
      await step('pageAfterRehighlight', () => page.hideHighlight())
      return steps
    `,
    probe: 'window.__facts()',
    effects: ({ playwright, neo }) => {
      assert.deepEqual(neo, playwright, 'browser effect')
      assert.deepEqual(playwright, {
        log: ['+main', '+child', '-main', '-child', '+main', '-main'],
        main: 0,
        child: 0,
      })
    },
  })
  assert.deepEqual(outcome.value, [
    ['locator', true],
    ['neverShown', true],
    ['frame', true],
    ['page', true],
    ['pageAgain', true],
    ['pageAfterRehighlight', true],
  ])
})
