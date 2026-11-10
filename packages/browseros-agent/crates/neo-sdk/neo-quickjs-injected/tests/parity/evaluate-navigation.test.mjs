// An evaluate sent while a navigation of its frame is pending, through the Playwright reference
// and through Neo. Chromium holds the call until the next document commits, then fails it with
// the old context, and Playwright rejects with its "Execution context was destroyed" error: the
// page function never runs again in a document the script did not target. An evaluate sent after
// the commit runs in the new document. Values by `describe` (values.mjs), errors by category and
// the words Playwright's message has.
//
// From med-pw-dom's probe (pw-dom-reads r2 evidence, eval-during-navigation): before this fix Neo
// answered the pending call from the next document (`/slow`).
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'
import { VALUES } from './values.mjs'

test('an evaluate during a pending navigation rejects; one after the commit runs there', {
  skip,
  timeout: 180_000,
}, async (t) => {
  await compareMethod(t, {
    html: '<!doctype html><title>old</title><h1>old</h1>',
    slowMs: 3000,
    source: `${VALUES}
      await page.evaluate(() => { location.href = '/slow' })
      // The navigation has started: its response is ${3000} ms away.
      await page.waitForTimeout(300)
      const during = await outcome(() => page.evaluate(() => location.pathname),
        ['Execution context was destroyed'])
      await page.waitForURL('**/slow')
      const after = await outcome(() => page.evaluate(() => location.pathname))
      return { during, after }
    `,
    probe: 'location.pathname',
    values({ playwright, neo }) {
      t.diagnostic(JSON.stringify({ playwright, neo }))
      assert.deepEqual(playwright.during, {
        error: 'Error',
        mentions: ['Execution context was destroyed'],
      })
      assert.deepEqual(playwright.after, { value: '/slow' })
      assert.deepEqual(neo, playwright)
    },
  })
})
