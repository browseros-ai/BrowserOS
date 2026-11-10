// Init scripts stay in their own context, through Neo and through Playwright on one BrowserOS
// neo: the run's own context's `addInitScript` never reaches a page of a context
// `browser.newContext()` created, and a created context's script reaches that context's pages,
// a `newPage()` included, and none of the run's own. Its `dispose()` removes it from the
// context's later documents. Each case disposes what it added, as Playwright's connection keeps
// its default context's scripts across cases.
//
// Playwright cases (pin 26137face): `tests/library/browsercontext-add-init-script.spec.ts`
// "IS-024 should work with browser context scripts @smoke" and "IS-029 should remove context
// init script after dispose", each run across two contexts.

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { skip } from './harness.mjs'
import { outcomeValue, withPlatform } from './platform.mjs'

const pages = {
  '/scripts.html': '<title>scripts</title><p>scripts</p>',
}

/** How many times each script ran in `p`'s current document. */
const READ = `const read = (p) => p.evaluate(() => ({ own: window.ownRuns ?? 0, created: window.createdRuns ?? 0 }))
`

const cases = [
  {
    name: "the run's own context's script stays out of a created context",
    source: `${READ}const created = await context.browser().newContext()
const script = await context.addInitScript(() => { window.ownRuns = (window.ownRuns ?? 0) + 1 })
const own = await context.newPage()
await own.goto(page.url())
const other = await created.newPage()
await other.goto(page.url())
const result = { own: await read(own), other: await read(other) }
await script.dispose()
await own.close()
await created.close()
return result`,
    expected: {
      own: { own: 1, created: 0 },
      other: { own: 0, created: 0 },
    },
  },
  {
    name: "a created context's script reaches its own pages, newPage's included, and is disposed",
    source: `${READ}const created = await context.browser().newContext()
const script = await created.addInitScript(() => { window.createdRuns = (window.createdRuns ?? 0) + 1 })
const fresh = await created.newPage()
await fresh.goto(page.url())
const navigated = await read(fresh)
const own = await context.newPage()
await own.goto(page.url())
const inOwn = await read(own)
await page.reload()
const inRunPage = await read(page)
await script.dispose()
await fresh.reload()
const afterDispose = await read(fresh)
await own.close()
await created.close()
return { navigated, inOwn, inRunPage, afterDispose }`,
    expected: {
      navigated: { own: 0, created: 1 },
      inOwn: { own: 0, created: 0 },
      inRunPage: { own: 0, created: 0 },
      afterDispose: { own: 0, created: 0 },
    },
  },
]

test('init scripts stay in their context: Neo matches Playwright', {
  skip,
}, async (t) => {
  await withPlatform(t, { pages }, async ({ playwright, neo }) => {
    for (const { name, source, expected } of cases) {
      const reference = outcomeValue(await playwright('/scripts.html', source))
      assert.deepEqual(reference, expected, `${name}: Playwright`)
      assert.deepEqual(
        outcomeValue(await neo('/scripts.html', source)),
        reference,
        `${name}: Neo`,
      )
    }
  })
})
