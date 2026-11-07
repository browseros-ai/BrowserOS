// playwright.devices through Neo and through Playwright on one BrowserOS neo: the same
// descriptors at the pin, one object across reads, and a new context spread from a descriptor
// emulating it (screen, device scale factor, user agent, touch), on tests/pages/emulation.html.
//
// Playwright: `playwright.devices` (`client/playwright.ts`, pin 26137face); the descriptor case
// follows `tests/library/browsercontext-viewport-mobile.spec.ts`'s use of `devices['iPhone 13']`.

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { skip } from './harness.mjs'
import { fixturePage, outcomeValue, withPlatform } from './platform.mjs'

const pages = { '/emulation.html': fixturePage('emulation.html') }

const source = `const devices = playwright.devices
const iphone = devices['iPhone 13']
const created = await context.browser().newContext({ ...iphone })
const other = await created.newPage()
await other.goto(page.url())
const s = await settle((s) => s.devicePixelRatio === iphone.deviceScaleFactor, other)
await created.close()
return {
  count: Object.keys(devices).length,
  same: playwright.devices === devices,
  iphone,
  desktop: devices['Desktop Chrome'].viewport,
  emulated: { screen: s.screen, dpr: s.devicePixelRatio, userAgent: s.userAgent === iphone.userAgent, touch: s.maxTouchPoints > 0 },
}`

test('playwright.devices: Neo matches Playwright', { skip }, async (t) => {
  await withPlatform(t, { pages }, async ({ playwright, neo }) => {
    const reference = outcomeValue(await playwright('/emulation.html', source))
    assert.equal(reference.same, true)
    assert.deepEqual(reference.emulated, {
      screen: [390, 844],
      dpr: 3,
      userAgent: true,
      touch: true,
    })
    assert.deepEqual(
      outcomeValue(await neo('/emulation.html', source)),
      reference,
    )
  })
})
