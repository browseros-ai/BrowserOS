// browser.contexts() and browser.browserType() through Neo and through Playwright on one
// BrowserOS neo: the script's own context first, a created context after it until it closes, the
// same objects every time, and the browser type that is `playwright.chromium`.
//
// Playwright cases (pin 26137face): `tests/library/browser.spec.ts` "BR-001 should return
// browserType" and "BR-006 should fire context event on newContext".

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { skip } from './harness.mjs'
import { outcomeValue, withPlatform } from './platform.mjs'

const source = `const browser = context.browser()
const events = []
browser.on('context', (created) => events.push(created))
const before = browser.contexts()
const created = await browser.newContext()
const during = browser.contexts()
await created.close()
const after = browser.contexts()
return {
  browserType: browser.browserType() === playwright.chromium,
  before: [before.length, before[0] === context],
  during: [during.length, during[0] === context, during[1] === created],
  event: [events.length, events[0] === created],
  after: [after.length, after.includes(created)],
}`

test('browser.contexts: Neo matches Playwright', { skip }, async (t) => {
  await withPlatform(t, { pages: {} }, async ({ playwright, neo }) => {
    const reference = outcomeValue(await playwright('/contexts', source))
    assert.deepEqual(reference, {
      browserType: true,
      before: [1, true],
      during: [2, true, true],
      event: [1, true],
      after: [1, false],
    })
    assert.deepEqual(outcomeValue(await neo('/contexts', source)), reference)
  })
})
