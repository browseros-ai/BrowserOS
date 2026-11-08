// Locator.selectText waits for the element, then for it to be visible unless `force`, and
// selects its text as Playwright's `InjectedScript.selectText` does: a textarea's or input's
// whole value, a plain div's contents as the document selection, a hidden input's value when
// forced. A display:none input is retried until it shows or the timeout passes, and a strict
// locator, a missing element at its timeout and an invalid selector reject by kind.
//
// From Playwright's specs at the pin (tests/page/elementhandle-select-text.spec.ts at
// 26137face): 'EH-102 should select textarea', 'EH-103 should select input', 'EH-104 should
// select plain div', 'EH-105 should timeout waiting for invisible element' and 'EH-106 should
// wait for visible'.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

test('selectText selects the text of the element it names as Playwright does', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/select-text.html', import.meta.url),
      'utf8',
    ),
    source: `
      const voids = [
        await page.locator('#textarea').selectText() === undefined,
        await page.locator('#input').selectText({ timeout: 5000 }) === undefined,
        await page.locator('#hidden-input').selectText({ force: true }) === undefined,
      ]
      await page.click('#reveal')
      voids.push(await page.locator('#later').selectText() === undefined)
      voids.push(await page.locator('#plain').selectText() === undefined)
      const failures = {}
      for (const [name, act] of [
        ['notVisible', () => page.locator('#hidden-input').selectText({ timeout: 1000 })],
        ['strict', () => page.locator('.dup').selectText()],
        ['missing', () => page.locator('#missing').selectText({ timeout: 500 })],
        ['invalid', () => page.locator('[').selectText()],
      ]) {
        try { await act(); failures[name] = 'resolved' } catch (error) { failures[name] = errorCategory(error) }
      }
      return { voids, failures }
    `,
    probe: 'window.__facts()',
    effects: ({ playwright, neo }) => {
      assert.deepEqual(neo, playwright, 'browser effect')
      // The div's text becomes the document selection; its focus() is a no-op (a div without
      // tabindex), so #later keeps the focus, and Chromium collapses its selection as the
      // document selection moves.
      assert.deepEqual(playwright, {
        textarea: [0, 10],
        input: [0, 10],
        hidden: [0, 12],
        later: [0, 0],
        selection: 'some value',
        active: 'later',
      })
    },
  })
  assert.deepEqual(outcome.value, {
    voids: [true, true, true, true, true],
    failures: {
      notVisible: 'TimeoutError',
      strict: 'Error:strict',
      missing: 'TimeoutError',
      invalid: 'Error',
    },
  })
})
