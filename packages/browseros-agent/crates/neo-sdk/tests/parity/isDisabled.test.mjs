// isDisabled is isEnabled's mirror (Playwright's `_elementState` with `disabled`): the same
// waiting and rejections, opposite values. Native, ARIA, fieldset and label-followed disabling,
// a plain div (not disabled), and a control inserted 500 ms after the `#later` click. Cases after
// `LO-065 isEnabled and isDisabled should work` (tests/page/locator-convenience.spec.ts at
// 26137face).
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

test('isDisabled mirrors isEnabled and waits for a match', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/element-editable.html', import.meta.url),
      'utf8',
    ),
    source: `
      const values = [await page.locator('#enabled-button').isDisabled(), await page.isDisabled('#disabled-button', null),
        await page.mainFrame().isDisabled('#plain', null), await page.locator('#aria-disabled').isDisabled(),
        await page.isDisabled('#fieldset-input'), await page.locator('#label').isDisabled()]
      await page.click('#later')
      const late = await page.isDisabled('#late')
      const all = [...values, late]
      if (!all.every(v => typeof v === 'boolean')) throw new Error('isDisabled must return booleans')
      const errors = []
      for (const [selector, options] of [['.dup'], ['#missing', { timeout: 500 }]]) {
        try { await page.locator(selector).isDisabled(options); errors.push('resolved') }
        catch (error) { errors.push(errorCategory(error)) }
      }
      return { values, late, types: all.map(v => typeof v), errors }
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(outcome.value, {
    values: [false, true, false, true, true, true],
    late: true,
    types: Array(7).fill('boolean'),
    errors: ['Error:strict', 'TimeoutError'],
  })
  const reads = outcome.calls.filter((c) => c.method === 'isDisabled')
  assert.equal(reads.length, 9)
  assert.ok(reads.every((c) => c.delivery === undefined))
})
