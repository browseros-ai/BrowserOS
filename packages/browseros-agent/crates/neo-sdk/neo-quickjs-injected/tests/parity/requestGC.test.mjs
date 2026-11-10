// page.requestGC through Neo and through Playwright on one BrowserOS neo, on fresh tabs of
// tests/pages/emulation.html, which holds one object only through a WeakRef and keeps another
// reachable from its global: before the call neither is collected, after it the unreachable one
// is gone and the reachable one stays.
//
// Playwright case (pin 26137face): `tests/page/page-request-gc.spec.ts` "RO-149 should work"
// (the page sets its objects up itself here, where Playwright's test evaluates them in).

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { skip } from './harness.mjs'
import { fixturePage, outcomeValue, withPlatform } from './platform.mjs'

const pages = { '/emulation.html': fixturePage('emulation.html') }

const source = `const before = (await state()).gc
const returned = await page.requestGC()
const after = (await settle((s) => s.gc.collected)).gc
return { before, returned: returned === undefined, after }`

test('requestGC: Neo matches Playwright', { skip }, async (t) => {
  await withPlatform(t, { pages }, async ({ playwright, neo }) => {
    const reference = outcomeValue(await playwright('/emulation.html', source))
    assert.deepEqual(reference, {
      before: { collected: false, kept: true },
      returned: true,
      after: { collected: true, kept: true },
    })
    assert.deepEqual(
      outcomeValue(await neo('/emulation.html', source)),
      reference,
    )
  })
})
