// setChecked delegates both requested states while preserving its public API and void result.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

test('setChecked returns undefined and applies both states on every receiver', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/check-actions.html', import.meta.url),
      'utf8',
    ),
    source: `
      const voids = [await page.setChecked('#unchecked', true, null) === undefined,
        await page.mainFrame().setChecked('#checked-frame', false, null) === undefined,
        await page.locator('#checked-locator').setChecked(false) === undefined,
        await page.locator('#aria').setChecked(true) === undefined,
        await page.locator('#unchecked').setChecked(false) === undefined]
      if (!voids.every(Boolean)) throw new Error('setChecked must return undefined')
      let wrong = 'resolved'
      try { await page.locator('#plain').setChecked(true) } catch (error) { wrong = errorCategory(error) }
      if (wrong !== 'Error') throw new Error('wrong element must reject')
      return { voids, wrong }
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(
    outcome.calls.map((c) => c.delivery),
    ['done', 'done', 'done', 'done', 'done', 'notSent'],
  )
})
