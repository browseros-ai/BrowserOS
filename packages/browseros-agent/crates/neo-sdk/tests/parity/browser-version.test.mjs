// browser.version() and isConnected() through Neo and through Playwright, both attached to one
// BrowserOS neo: the same version string, in Chromium's four-part shape, and a connected browser
// whose first context is the script's own.
//
// Playwright case (pin 26137face): `tests/library/browser.spec.ts` "BR-004 version should work".

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { skip } from './harness.mjs'
import { outcomeValue, withPlatform } from './platform.mjs'

const source = `const browser = context.browser()
const version = browser.version()
return { version, type: typeof version, shape: /^\\d+\\.\\d+\\.\\d+\\.\\d+$/.test(version),
  connected: browser.isConnected(), own: browser.contexts()[0] === context }`

test('browser.version: Neo matches Playwright', { skip }, async (t) => {
  await withPlatform(t, { pages: {} }, async ({ playwright, neo }) => {
    const reference = outcomeValue(await playwright('/version', source))
    assert.equal(
      reference.shape,
      true,
      'Playwright reports a four-part version',
    )
    assert.deepEqual(outcomeValue(await neo('/version', source)), reference)
  })
})
