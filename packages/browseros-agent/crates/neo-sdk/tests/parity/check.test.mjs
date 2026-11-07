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
      const voids = [await page.check('#unchecked', null) === undefined,
        await page.mainFrame().check('#radio', null) === undefined,
        await page.locator('#aria').check() === undefined,
        await page.locator('#checked').check() === undefined,
        await page.locator('#trial').check({trial:true}) === undefined,
        await page.locator('#replaced').check() === undefined]
      if (voids.length !== 6 || !voids.every(Boolean)) throw new Error('check must return undefined')
      let wrong = 'resolved'
      try { await page.locator('#plain').check() } catch (error) { wrong = errorCategory(error) }
      if (wrong !== 'Error') throw new Error('wrong element must reject')
      let timeout = 'resolved'
      try { await page.locator('#covered').check({timeout:500}) } catch (error) { timeout = errorCategory(error) }
      if (timeout !== 'TimeoutError') throw new Error('covered check must time out')
      return { voids, wrong, timeout }
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(
    outcome.calls.map((c) => c.delivery),
    [
      'done',
      'done',
      'done',
      'notSent',
      'notSent',
      'done',
      'notSent',
      'notSent',
    ],
  )
})
