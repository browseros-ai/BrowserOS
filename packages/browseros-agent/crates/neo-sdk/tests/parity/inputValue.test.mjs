// inputValue is a live read on Page, Frame, Locator and label targets.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

test('inputValue returns strings and rejects wrong or ambiguous targets', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/form-actions.html', import.meta.url),
      'utf8',
    ),
    source: `
      const values = [await page.inputValue('#text'), await page.mainFrame().inputValue('#area'),
        await page.locator('#single').inputValue(), await page.locator('label').inputValue()]
      if (!values.every(v => typeof v === 'string')) throw new Error('inputValue must return strings')
      const errors = []
      for (const selector of ['#plain', '.duplicate']) {
        try { await page.locator(selector).inputValue(); errors.push('resolved') }
        catch (error) { errors.push(error.name) }
      }
      if (!errors.every(e => e === 'Error')) throw new Error('wrong and strict targets must reject')
      let invalid = 'resolved'
      try { await page.inputValue(123) } catch (error) { invalid = error.name }
      if (invalid !== 'Error') throw new Error('invalid selector type must reject')
      return { values, types: values.map(v => typeof v), errors, invalid }
    `,
    probe: 'window.__facts()',
  })
  assert.ok(outcome.calls.every((c) => c.delivery === undefined))
})
