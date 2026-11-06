// Fill's Page, Frame and Locator receivers share one behavior check on fresh pages.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

test('fill returns undefined and uses ordinary input events', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/form-actions.html', import.meta.url),
      'utf8',
    ),
    source: `
      const voids = [
        await page.fill('#text', 'Page', null) === undefined,
        await page.mainFrame().fill('#area', 'Frame', null) === undefined,
        await page.getByLabel('Name').fill('Locator') === undefined,
        await page.locator('#editable').fill('Editable') === undefined,
        await page.locator('#date').fill('2026-11-06') === undefined,
      ]
      if (voids.length !== 5 || !voids.every(Boolean)) throw new Error('fill must return undefined')
      let invalid = 'resolved'
      try { await page.locator('#text').fill(123) } catch (error) { invalid = errorCategory(error) }
      if (invalid !== 'error') throw new Error('invalid fill value must reject')
      let timeout = 'resolved'
      try { await page.locator('#disabled').fill('x', {timeout:500}) } catch (error) { timeout = errorCategory(error) }
      if (timeout !== 'timeout') throw new Error('disabled fill must time out')
      return { voids, invalid, timeout }
    `,
    probe: 'window.__facts()',
  })
  const delivered = outcome.calls.filter((c) => c.result.kind === 'ok')
  assert.equal(delivered.length, 5)
  assert.ok(delivered.every((c) => c.delivery === 'done'))
})
