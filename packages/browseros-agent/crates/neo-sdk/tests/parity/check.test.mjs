// Check compares real clicks, radio/ARIA targets, an existing state and trial behavior.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

test('check returns undefined and sets checked state on every receiver', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/check-actions.html', import.meta.url),
      'utf8',
    ),
    source: `
      const voids = [await page.check('#unchecked') === undefined,
        await page.mainFrame().check('#radio') === undefined,
        await page.locator('#aria').check() === undefined,
        await page.locator('#checked').check() === undefined,
        await page.locator('#trial').check({trial:true}) === undefined]
      if (!voids.every(Boolean)) throw new Error('check must return undefined')
      let wrong = 'resolved'
      try { await page.locator('#plain').check() } catch (error) { wrong = error.name }
      if (wrong !== 'Error') throw new Error('wrong element must reject')
      return { voids, wrong }
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(
    outcome.calls.map((c) => c.delivery),
    ['done', 'done', 'done', 'notSent', 'notSent', 'notSent'],
  )
})
