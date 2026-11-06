// selectOption compares value/label/index matching, arrays, deselection and argument rejection.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

test('selectOption returns selected values for every receiver', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/form-actions.html', import.meta.url),
      'utf8',
    ),
    source: `
      const values = [await page.selectOption('#single', 'two'),
        await page.mainFrame().selectOption('#single', { label: 'First' }),
        await page.locator('#single').selectOption({ index: 1 }),
        await page.locator('#single').selectOption({ value: 'one' }),
        await page.locator('#multiple').selectOption(['three', 'one']),
        await page.locator('#single').selectOption(null)]
      if (!values.every(v => Array.isArray(v) && v.every(s => typeof s === 'string')))
        throw new Error('selectOption must return string arrays')
      let invalid = 'resolved'
      try { await page.locator('#single').selectOption({ index: 'bad' }) } catch (error) { invalid = error.name }
      if (invalid !== 'Error') throw new Error('invalid option index must reject')
      return { values, invalid }
    `,
    probe: 'window.__facts()',
  })
  assert.ok(
    outcome.calls
      .filter((c) => c.result.kind === 'ok')
      .every((c) => c.delivery === 'done'),
  )
})
