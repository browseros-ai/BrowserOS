// browser.removeAllListeners through Neo and through Playwright on one BrowserOS neo: removing a
// browser event's listeners stops them (a context created after does not reach them), a listener
// added later hears the next one, and with `behavior` it answers a promise that settles once
// running listeners have. (Not `disconnected`: Playwright's own `close()` waits on a listener of
// its own there, which removing them all would drop.)
//
// Playwright: `client/eventEmitter.ts` `EventEmitter.removeAllListeners` (pin 26137face), which
// `Browser` inherits; the `context` event is `tests/library/browser.spec.ts` "BR-006 should fire
// context event on newContext".

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { skip } from './harness.mjs'
import { outcomeValue, withPlatform } from './platform.mjs'

const source = `const browser = context.browser()
const seen = []
browser.on('context', () => seen.push('removed'))
const returned = browser.removeAllListeners('context')
const first = await browser.newContext()
await first.close()
browser.on('context', () => seen.push('kept'))
const second = await browser.newContext()
await second.close()
const waited = await browser.removeAllListeners('context', { behavior: 'wait' })
const third = await browser.newContext()
await third.close()
return {
  returnsBrowser: returned === browser,
  waited: waited === undefined,
  seen,
  count: browser.listenerCount('context'),
}`

test('browser.removeAllListeners: Neo matches Playwright', {
  skip,
}, async (t) => {
  await withPlatform(t, { pages: {} }, async ({ playwright, neo }) => {
    const reference = outcomeValue(await playwright('/listeners', source))
    assert.deepEqual(reference, {
      returnsBrowser: true,
      waited: true,
      seen: ['kept'],
      count: 0,
    })
    assert.deepEqual(outcomeValue(await neo('/listeners', source)), reference)
  })
})
