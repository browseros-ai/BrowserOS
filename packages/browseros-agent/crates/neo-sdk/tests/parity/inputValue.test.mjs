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
      const values = [await page.inputValue('#text', null), await page.mainFrame().inputValue('#area', null),
        await page.locator('#single').inputValue(), await page.locator('label').inputValue()]
      if (values.length !== 4 || !values.every(v => typeof v === 'string')) throw new Error('inputValue must return strings')
      const errors = []
      for (const selector of ['#plain', '.duplicate']) {
        try { await page.locator(selector).inputValue(); errors.push('resolved') }
        catch (error) { errors.push(errorCategory(error)) }
      }
      if (errors.length !== 2 || errors[0] !== 'error' || errors[1] !== 'strict') throw new Error('wrong and strict targets must reject with their own categories')
      let invalid = 'resolved'
      try { await page.inputValue(123) } catch (error) { invalid = errorCategory(error) }
      if (invalid !== 'error') throw new Error('invalid selector type must reject')
      let timeout = 'resolved'
      try { await page.locator('#missing').inputValue({timeout:500}) } catch (error) { timeout = errorCategory(error) }
      if (timeout !== 'timeout') throw new Error('missing inputValue must time out')
      return { values, types: values.map(v => typeof v), errors, invalid, timeout }
    `,
    probe: 'window.__facts()',
  })
  assert.equal(outcome.calls.length, 8)
  assert.ok(outcome.calls.every((c) => c.delivery === undefined))
})
