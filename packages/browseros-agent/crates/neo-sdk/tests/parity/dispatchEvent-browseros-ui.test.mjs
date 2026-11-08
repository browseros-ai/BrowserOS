// dispatchEvent on a page carrying BrowserOS's takeover bar (browseros-ui.mjs). Playwright
// resolves its selector in the page's main world, where Neo hides the bar's root only for the
// query: a strict locator with one match on the page, `last()` and the page's own shadow root
// reach the page's elements, never the bar's, with the bar hidden and shown, as Playwright's do
// on a browser without it. The page's listeners record what each click reached.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareWithBar } from './browseros-ui.mjs'
import { skip } from './harness.mjs'

const source = `
  page.setDefaultTimeout(2000)
  const settle = (call) => call.then(() => 'resolved', (error) => errorCategory(error))
  return {
    strictBody: await settle(page.locator('body').dispatchEvent('ping')),
    lastButton: await settle(page.locator('button').last().dispatchEvent('click')),
    lightInput: await settle(page.locator('input').first().dispatchEvent('click')),
    textbox: await settle(page.getByRole('textbox').nth(1).dispatchEvent('click')),
    strictTwo: await settle(page.locator('input').dispatchEvent('click')),
    // The page keeps its own getter once the query is done.
    getterIntact: await page.evaluate(() =>
      String(Object.getOwnPropertyDescriptor(Element.prototype, 'shadowRoot').get).includes('[native code]')),
  }
`

test("dispatchEvent reaches the page's elements, never the takeover bar's", {
  skip,
  timeout: 300_000,
}, async (t) => {
  const { playwright, hidden, shown } = await compareWithBar(t, {
    source,
    probe: 'window.__clicks',
  })
  assert.deepEqual(playwright.value, {
    strictBody: 'resolved',
    lastButton: 'resolved',
    lightInput: 'resolved',
    textbox: 'resolved',
    strictTwo: 'Error:strict',
    getterIntact: true,
  })
  assert.deepEqual(playwright.probe, ['inner-go', 'name', 'inner-name'])
  for (const [bar, neo] of Object.entries({ hidden, shown })) {
    assert.deepEqual(
      neo.value,
      playwright.value,
      `bar ${bar}: ${JSON.stringify(neo.error)}`,
    )
    assert.deepEqual(
      neo.probe,
      playwright.probe,
      `bar ${bar}: what each click reached`,
    )
  }
})
