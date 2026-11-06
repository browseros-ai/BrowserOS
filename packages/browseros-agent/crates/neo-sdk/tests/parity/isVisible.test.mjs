// isVisible reads once on Page, Frame and Locator: no waiting, no timeout, the first match unless
// strict. `#appears` is shown 500 ms after the `#later` click, with `#late` inserted in the same
// task; textContent waits for `#late`, so the read after it sees `#appears` shown.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

test('isVisible returns booleans at once and rejects ambiguous or invalid targets', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/element-states.html', import.meta.url),
      'utf8',
    ),
    source: `
      const values = [await page.locator('#visible').isVisible(), await page.isVisible('#display-none'),
        await page.mainFrame().isVisible('#visibility-hidden'), await page.locator('#zero-size').isVisible(),
        await page.isVisible('#missing'), await page.isVisible('#visible', { timeout: 1 }),
        await page.isVisible('.dup')]
      await page.click('#later')
      const atOnce = await page.isVisible('#appears')
      await page.textContent('#late')
      const later = await page.isVisible('#appears')
      const all = [...values, atOnce, later]
      if (!all.every(v => typeof v === 'boolean')) throw new Error('isVisible must return booleans')
      let strict = 'resolved'
      try { await page.locator('.dup').isVisible() } catch (error) { strict = errorCategory(error) }
      let invalid = 'resolved'
      try { await page.isVisible(123) } catch (error) { invalid = errorCategory(error) }
      return { values, atOnce, later, types: all.map(v => typeof v), strict, invalid }
    `,
    probe: 'window.__facts()',
  })
  // Playwright sends isVisible without a timeout, so `{ timeout: 1 }` must not fire.
  assert.deepEqual(outcome.value, {
    values: [true, false, false, false, false, true, true],
    atOnce: false,
    later: true,
    types: Array(9).fill('boolean'),
    strict: 'strict',
    invalid: 'error',
  })
  const reads = outcome.calls.filter((c) => c.method === 'isVisible')
  assert.equal(reads.length, 11)
  assert.ok(reads.every((c) => c.delivery === undefined))
})
