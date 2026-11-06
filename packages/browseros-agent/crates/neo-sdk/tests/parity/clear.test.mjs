// Playwright exposes clear on Locator; all editable targets use the fill('') input path.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

test('clear returns undefined and empties editable targets', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/form-actions.html', import.meta.url),
      'utf8',
    ),
    source: `
      const voids = []
      for (const selector of ['#text', '#area', '#editable'])
        voids.push(await page.locator(selector).clear() === undefined)
      if (voids.length !== 3 || !voids.every(Boolean)) throw new Error('clear must return undefined')
      let invalid = 'resolved'
      try { await page.locator('#text').clear({ force: 'yes' }) } catch (error) { invalid = errorCategory(error) }
      if (invalid !== 'error') throw new Error('invalid clear force must reject')
      return { voids, invalid }
    `,
    probe: 'window.__facts()',
  })
  const delivered = outcome.calls.filter((c) => c.result.kind === 'ok')
  assert.equal(delivered.length, 3)
  assert.ok(delivered.every((c) => c.delivery === 'done'))
})
