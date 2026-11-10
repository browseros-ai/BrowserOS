// Regression: client/browser.ts Browser.close and Browser._didClose. Pending context
// creations must not repopulate the browser after its connection has closed.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { skip } from './harness.mjs'
import { outcomeValue, withPlatform } from './platform.mjs'

test('browser.close rejects pending context creations', { skip }, async (t) => {
  await withPlatform(t, {}, async ({ playwright, neo, cdp }) => {
    const source = `const browser = context.browser()
const creating = Array.from({length:4}, () => browser.newContext().catch(() => null))
await browser.close()
await Promise.all(creating)
return { connected:browser.isConnected(), contexts:browser.contexts().length }`
    const reference = outcomeValue(await playwright('/close-race', source))
    const actual = outcomeValue(await neo('/close-race', source))
    t.diagnostic(JSON.stringify({ reference, actual }))
    assert.deepEqual(reference, { connected: false, contexts: 0 })
    assert.deepEqual(actual, reference)
    assert.deepEqual(
      (await cdp.send('Target.getBrowserContexts')).browserContextIds,
      [],
    )
  })
})
