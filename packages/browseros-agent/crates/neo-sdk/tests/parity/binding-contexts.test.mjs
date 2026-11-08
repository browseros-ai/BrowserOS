// Bindings stay in their own context, through Neo and through Playwright on one BrowserOS neo:
// the run's own context's `exposeFunction` answers the run's own pages and never reaches a page
// of a context `browser.newContext()` created, one opened before it or after. A binding on a
// created context, or on one of its pages, is asserted on each engine's side: Playwright serves
// it, Neo fails naming the method (neo-runtime `browser_calls.rs` `created_state`). Each case
// disconnects Playwright before Neo's turn, so its bindings go with its sessions.
//
// Playwright cases (pin 26137face): `tests/library/browsercontext-expose-function.spec.ts`
// "EF-023 should work" and "EF-024 should dispose", run across two contexts.

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { skip } from './harness.mjs'
import { outcomeValue, withPlatform } from './platform.mjs'

const pages = { '/bindings.html': '<title>bindings</title><p>bindings</p>' }

const SCOPED = `const created = await context.browser().newContext()
const before = await created.newPage()
await before.goto(page.url())
const fn = await context.exposeFunction('ownFn', (a, b) => a + b)
await page.reload()
const inOwn = await page.evaluate(() => window.ownFn(2, 40))
await before.reload()
const inBefore = await before.evaluate(() => typeof window.ownFn)
const after = await created.newPage()
await after.goto(page.url())
const inAfter = await after.evaluate(() => typeof window.ownFn)
await fn.dispose()
await created.close()
return { inOwn, inBefore, inAfter }`

const ON_CREATED = `const created = await context.browser().newContext()
const other = await created.newPage()
await other.goto(page.url())
const first = (call) => call.then(() => 'answered', (error) => ['failed', error.message.split('\\n')[0]])
const result = {
  context: await first(created.exposeFunction('createdFn', () => 1)),
  page: await first(other.exposeFunction('pageFn', () => 2)),
}
await created.close()
return result`

test('bindings stay in their context: Neo matches Playwright', {
  skip,
}, async (t) => {
  await withPlatform(t, { pages }, async ({ playwright, neo, connect }) => {
    const reference = outcomeValue(await playwright('/bindings.html', SCOPED))
    await (await connect()).close()
    assert.deepEqual(
      reference,
      { inOwn: 42, inBefore: 'undefined', inAfter: 'undefined' },
      'Playwright',
    )
    assert.deepEqual(
      outcomeValue(await neo('/bindings.html', SCOPED)),
      reference,
      'Neo',
    )
  })
})

test('a binding on a created context: Playwright serves it, Neo refuses naming it', {
  skip,
}, async (t) => {
  await withPlatform(t, { pages }, async ({ playwright, neo, connect }) => {
    assert.deepEqual(
      outcomeValue(await playwright('/bindings.html', ON_CREATED)),
      { context: 'answered', page: 'answered' },
      'Playwright',
    )
    await (await connect()).close()
    const refused = (api) => [
      'failed',
      `${api}: neo-runtime: exposeBinding is not served yet for a context browser.newContext created, only for the run's own context`,
    ]
    assert.deepEqual(
      outcomeValue(await neo('/bindings.html', ON_CREATED)),
      {
        context: refused('browserContext.exposeFunction'),
        page: refused('page.exposeFunction'),
      },
      'Neo',
    )
  })
})
