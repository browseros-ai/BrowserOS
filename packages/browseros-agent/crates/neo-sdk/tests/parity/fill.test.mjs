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
        await page.fill('#text', 'Page') === undefined,
        await page.mainFrame().fill('#area', 'Frame') === undefined,
        await page.getByLabel('Name').fill('Locator') === undefined,
        await page.locator('#editable').fill('Editable') === undefined,
        await page.locator('#date').fill('2026-11-06') === undefined,
      ]
      if (!voids.every(Boolean)) throw new Error('fill must return undefined')
      let invalid = 'resolved'
      try { await page.locator('#text').fill(123) } catch (error) { invalid = error.name }
      if (invalid !== 'Error') throw new Error('invalid fill value must reject')
      return { voids, invalid }
    `,
    probe: 'window.__facts()',
  })
  assert.ok(
    outcome.calls
      .filter((c) => c.result.kind === 'ok')
      .every((c) => c.delivery === 'done'),
  )
})
