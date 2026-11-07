// browser.close() through Neo and through Playwright, each attached to one BrowserOS neo: the
// close events fire in Playwright's order (every open page, the created context, the script's
// own context, then `disconnected`), every object then reads closed and every call fails as on a
// closed browser, closing again resolves, and the browser itself keeps running with the script's
// own tab open.
//
// Playwright case (pin 26137face): `tests/library/browser.spec.ts` "BR-005 should dispatch
// page.on(close) upon browser.close and reject evaluate" (on a connected browser here, so the
// browser survives the close as Playwright's connected close leaves it).

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { skip } from './harness.mjs'
import { outcomeValue, withPlatform } from './platform.mjs'

const source = `const browser = context.browser()
const events = []
browser.on('disconnected', (b) => events.push(['disconnected', b === browser]))
context.on('close', () => events.push('own context close'))
page.on('close', () => events.push('page close'))
const created = await browser.newContext()
const createdPage = await created.newPage()
created.on('close', () => events.push('created context close'))
createdPage.on('close', () => events.push('created page close'))
const returned = await browser.close()
const again = await browser.close()
return {
  events,
  returned: [returned === undefined, again === undefined],
  connected: browser.isConnected(),
  contexts: browser.contexts().length,
  pages: [context.pages().length, created.pages().length],
  closed: [page.isClosed(), createdPage.isClosed()],
  calls: [
    await page.title().then(() => 'resolved', errorCategory),
    await page.locator('h1').click().then(() => 'resolved', errorCategory),
    await context.newPage().then(() => 'resolved', errorCategory),
    await browser.newContext().then(() => 'resolved', errorCategory),
  ],
  version: typeof browser.version(),
}`

test('browser.close: Neo matches Playwright', { skip }, async (t) => {
  await withPlatform(t, { pages: {} }, async ({ playwright, neo, cdp }) => {
    const reference = outcomeValue(await playwright('/close', source))
    assert.deepEqual(reference, {
      events: [
        'page close',
        'created page close',
        'created context close',
        'own context close',
        ['disconnected', true],
      ],
      returned: [true, true],
      connected: false,
      contexts: 0,
      pages: [0, 0],
      closed: [true, true],
      calls: ['Error', 'Error', 'Error', 'Error'],
      version: 'string',
    })
    const actual = await neo('/close', source)
    assert.deepEqual(outcomeValue(actual), reference)
    // BrowserOS keeps running, with the run's own tab open and no context of the run left.
    const { targetInfos } = await cdp.send('Target.getTargets')
    assert.ok(
      targetInfos.some((target) => target.targetId === actual.targetId),
      "the run's own tab stays open",
    )
    const { browserContextIds } = await cdp.send('Target.getBrowserContexts')
    assert.deepEqual(browserContextIds, [])
  })
})
