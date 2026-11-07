// browser.removeAllListeners through Neo and through Playwright on one BrowserOS neo: removing a
// browser event's listeners stops them (a context created after does not reach them), others
// stay, and with `behavior` it answers a promise that settles once running listeners have.
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
browser.on('disconnected', () => seen.push('disconnected'))
const returned = browser.removeAllListeners('context')
const first = await browser.newContext()
await first.close()
browser.on('context', () => seen.push('kept'))
const waited = await browser.removeAllListeners('disconnected', { behavior: 'wait' })
const second = await browser.newContext()
await second.close()
return {
  returnsBrowser: returned === browser,
  waited: waited === undefined,
  seen,
  counts: [browser.listenerCount('context'), browser.listenerCount('disconnected')],
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
      counts: [1, 0],
    })
    assert.deepEqual(outcomeValue(await neo('/listeners', source)), reference)
  })
})
