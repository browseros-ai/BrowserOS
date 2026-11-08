// isChecked on Page, Frame and Locator: native, label and ARIA targets (`mixed` reads false), a
// wait for a checkbox inserted 500 ms after the `#later` click, re-reads after check/uncheck, and
// rejections by kind: a wrong element at once, a strict match, a missing one at its timeout.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

test('isChecked returns booleans, waits for a match and rejects wrong or ambiguous targets', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/element-states.html', import.meta.url),
      'utf8',
    ),
    source: `
      const values = [await page.isChecked('#unchecked', null), await page.mainFrame().isChecked('#radio', null),
        await page.locator('#label').isChecked(), await page.locator('#aria-true').isChecked(),
        await page.isChecked('#aria-mixed')]
      await page.click('#later')
      const late = await page.isChecked('#late')
      await page.check('#unchecked')
      await page.locator('#labelled').uncheck()
      const flipped = [await page.isChecked('#unchecked'), await page.locator('#label').isChecked()]
      const all = [...values, late, ...flipped]
      if (!all.every(v => typeof v === 'boolean')) throw new Error('isChecked must return booleans')
      const errors = []
      for (const [selector, options] of [['#plain'], ['.dup'], ['#missing', { timeout: 500 }]]) {
        try { await page.locator(selector).isChecked(options); errors.push('resolved') }
        catch (error) { errors.push(errorCategory(error)) }
      }
      return { values, late, flipped, types: all.map(v => typeof v), errors }
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(outcome.value, {
    values: [false, true, true, true, false],
    late: true,
    flipped: [true, false],
    types: Array(8).fill('boolean'),
    errors: ['Error', 'Error:strict', 'TimeoutError'],
  })
  const reads = outcome.calls.filter((c) => c.method === 'isChecked')
  assert.equal(reads.length, 11)
  assert.ok(reads.every((c) => c.delivery === undefined))
})
