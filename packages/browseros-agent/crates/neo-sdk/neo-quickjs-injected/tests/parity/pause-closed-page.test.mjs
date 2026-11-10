// Regression: client/page.ts Page.pause races the page's closed/crashed scope.
// A debugger-held pause must settle when its page closes, without requiring resume.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { skip } from './harness.mjs'
import { outcomeValue, withPlatform } from './platform.mjs'

test('pause settles when its page closes', { skip }, async (t) => {
  await withPlatform(t, {}, async ({ playwright, neo }) => {
    const source = `const dbg = context.debugger
await dbg.requestPause()
const changed = new Promise(resolve => dbg.once('pausedstatechanged', resolve))
const paused = page.pause()
await changed
await page.close()
const settled = await Promise.race([
  paused.then(() => 'resolved', e => e.name),
  new Promise(resolve => setTimeout(() => resolve('still pending'), 2000)),
])
await dbg.resume().catch(() => {})
await paused
return settled`
    const reference = outcomeValue(await playwright('/pause-close', source))
    const actual = outcomeValue(await neo('/pause-close', source))
    assert.equal(reference, 'resolved')
    assert.equal(actual, reference, 'closing the page must release its pause')
  })
})
