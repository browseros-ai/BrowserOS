// playwright.errors through Neo and through Playwright on one BrowserOS neo: it holds
// `TimeoutError`, a real timeout of a locator action and of an event wait is an instance of it
// (and of Error), and an error of another kind is not.
//
// Playwright: the public `errors` (`client/playwright.ts`, pin 26137face); the timeouts come from
// the same calls in both engines.

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { skip } from './harness.mjs'
import { outcomeValue, withPlatform } from './platform.mjs'

const pages = { '/errors.html': '<title>errors</title><p>nothing to click</p>' }

const source = `const { TimeoutError } = playwright.errors
const describe = (e) => [e instanceof TimeoutError, e instanceof Error, e.name]
const click = await page.locator('#missing').click({ timeout: 150 }).catch((e) => e)
const wait = await page.waitForEvent('console', { timeout: 150 }).catch((e) => e)
const other = await page.emulateMedia({ media: 'bad' }).catch((e) => e)
return { keys: Object.keys(playwright.errors), click: describe(click), wait: describe(wait), other: describe(other) }`

test('playwright.errors: Neo matches Playwright', { skip }, async (t) => {
  await withPlatform(t, { pages }, async ({ playwright, neo }) => {
    const reference = outcomeValue(await playwright('/errors.html', source))
    assert.deepEqual(reference, {
      keys: ['TimeoutError'],
      click: [true, true, 'TimeoutError'],
      wait: [true, true, 'TimeoutError'],
      other: [false, true, 'Error'],
    })
    assert.deepEqual(outcomeValue(await neo('/errors.html', source)), reference)
  })
})
