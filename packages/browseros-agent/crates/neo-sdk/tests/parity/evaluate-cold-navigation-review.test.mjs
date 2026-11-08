// A first evaluation must keep the pending navigation's original document, without a warm helper.
// Wait for the real navigation request; the fixture delays its response. Compare public values
// and persistent page effects, without matching error prose or measuring elapsed time.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

test('first evaluate during a pending goto rejects without running in the next document', {
  skip,
  timeout: 180000,
}, async (t) => {
  await compareMethod(t, {
    html: '<!doctype html><title>old</title><script>sessionStorage.setItem("cold_runs", "0")</script>',
    slowMs: 3000,
    source: `
      const old = page.url()
      const requested = page.waitForRequest('**/slow', { timeout: 10000 })
      const navigation = page.goto(old.slice(0, old.lastIndexOf('/')) + '/slow')
      await requested
      const during = await page.evaluate(() => {
        sessionStorage.setItem('cold_runs', String(Number(sessionStorage.getItem('cold_runs')) + 1))
        return location.pathname
      }).then(value => ({ value }), error => ({ error: error.name }))
      await navigation
      const after = await page.evaluate(() => ({
        path: location.pathname, runs: Number(sessionStorage.getItem('cold_runs')),
      }))
      return { during, after }
    `,
    probe:
      '({ path: location.pathname, runs: Number(sessionStorage.getItem("cold_runs")) })',
    values({ playwright, neo }) {
      t.diagnostic(JSON.stringify({ playwright, neo }))
      assert.deepEqual(playwright, {
        during: { error: 'Error' },
        after: { path: '/slow', runs: 0 },
      })
      assert.deepEqual(
        neo,
        playwright,
        'the first evaluate must not execute in the next document',
      )
    },
  })
})
