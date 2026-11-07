// page.viewportSize() through Neo and through Playwright on one BrowserOS neo: `null` on a tab of
// the connected (run's own) context, which emulates no viewport, the size once
// `setViewportSize` set one, Playwright's 1280x720 default on a page of a new context, the
// context's own `viewport`, and `null` again with `viewport: null`. Each new-context page's
// innerWidth is read too, so the answer matches the page.
//
// Playwright cases (pin 26137face): `tests/library/browsercontext-viewport.spec.ts`
// "VP-001 should get the proper default viewport size" and "VP-002 should set the proper
// viewport size".

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { skip } from './harness.mjs'
import { fixturePage, outcomeValue, withPlatform } from './platform.mjs'

const pages = { '/emulation.html': fixturePage('emulation.html') }

const source = `const own = page.viewportSize()
await page.setViewportSize({ width: 640, height: 480 })
const set = page.viewportSize()
const url = page.url()
const opened = []
for (const options of [{}, { viewport: { width: 456, height: 345 } }, { viewport: null }]) {
  const created = await context.browser().newContext(options)
  const other = await created.newPage()
  await other.goto(url)
  const width = options.viewport === null
    ? null
    : (await settle((s) => s.innerWidth === (options.viewport?.width ?? 1280), other)).innerWidth
  opened.push({ viewport: other.viewportSize(), width })
  await created.close()
}
return { own, set, opened }`

test('viewportSize: Neo matches Playwright', { skip }, async (t) => {
  await withPlatform(t, { pages }, async ({ playwright, neo }) => {
    const reference = outcomeValue(await playwright('/emulation.html', source))
    assert.deepEqual(reference, {
      own: null,
      set: { width: 640, height: 480 },
      opened: [
        { viewport: { width: 1280, height: 720 }, width: 1280 },
        { viewport: { width: 456, height: 345 }, width: 456 },
        { viewport: null, width: null },
      ],
    })
    assert.deepEqual(
      outcomeValue(await neo('/emulation.html', source)),
      reference,
    )
  })
})
