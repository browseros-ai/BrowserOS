// Uncheck compares Page, Frame, Locator and ARIA changes, plus an already-unchecked no-op.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

test('uncheck returns undefined and clears checked state on every receiver', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/check-actions.html', import.meta.url),
      'utf8',
    ),
    source: `
      const voids = [await page.uncheck('#checked', null) === undefined,
        await page.mainFrame().uncheck('#checked-frame', null) === undefined,
        await page.locator('#checked-locator').uncheck() === undefined,
        await page.locator('#aria-checked').uncheck() === undefined,
        await page.locator('#unchecked').uncheck() === undefined]
      if (!voids.every(Boolean)) throw new Error('uncheck must return undefined')
      let wrong = 'resolved'
      try { await page.locator('#plain').uncheck() } catch (error) { wrong = errorCategory(error) }
      if (wrong !== 'Error') throw new Error('wrong element must reject')
      return { voids, wrong }
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(
    outcome.calls.map((c) => c.delivery),
    ['done', 'done', 'done', 'done', 'notSent', 'notSent'],
  )
})
