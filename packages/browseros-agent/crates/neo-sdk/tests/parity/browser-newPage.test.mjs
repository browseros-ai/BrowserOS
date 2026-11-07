// browser.newPage through Neo and through Playwright on one BrowserOS neo: each page gets a
// context of its own, which closes with the page, and that context refuses a second page.
//
// Playwright cases (pin 26137face): `tests/library/browser.spec.ts` "BR-002 should create new
// page" (counted from the connected browser's own context, so one more than Playwright's
// launched browser) and "BR-003 should throw upon second create new page".

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { skip } from './harness.mjs'
import { outcomeValue, withPlatform } from './platform.mjs'

const source = `const browser = context.browser()
const page1 = await browser.newPage()
const one = browser.contexts().length
const page2 = await browser.newPage()
const two = browser.contexts().length
const second = await page1.context().newPage().then(() => 'created', (e) => [errorCategory(e), e.message.includes('Please use browser.newContext()')])
const own = [page1.context() !== context, page1.context() !== page2.context(), page1.context().pages().length]
await page1.close()
const three = browser.contexts().length
await page2.close()
const four = browser.contexts().length
return { counts: [one, two, three, four], second, own, closed: [page1.isClosed(), page2.isClosed()] }`

test('browser.newPage: Neo matches Playwright', { skip }, async (t) => {
  await withPlatform(t, { pages: {} }, async ({ playwright, neo }) => {
    const reference = outcomeValue(await playwright('/new-page', source))
    assert.deepEqual(reference, {
      counts: [2, 3, 2, 1],
      second: ['Error', true],
      own: [true, true, 1],
      closed: [true, true],
    })
    assert.deepEqual(outcomeValue(await neo('/new-page', source)), reference)
  })
})
