// ariaSnapshot on a page carrying BrowserOS's takeover bar (browseros-ui.mjs): Neo on BrowserOS
// as shipped, the bar in place, against Playwright on a browser without it. The REAL bench's
// `page.locator('body').ariaSnapshot({ mode: 'ai' })` (a strict mode violation before: the bar's
// own <body> matched too) answers the page's snapshot, refs included, with no node of the bar;
// the page's own shadow root still shows. Hidden bar, then shown bar.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareWithBar } from './browseros-ui.mjs'
import { skip } from './harness.mjs'

const source = `
  page.setDefaultTimeout(2000)
  const ai = await page.locator('body').ariaSnapshot({ mode: 'ai' })
  // A ref resolves against the latest snapshot, so it is read before the next one.
  const ref = /textbox "Inner name" \\[ref=(e\\d+)\\]/.exec(ai)?.[1]
  const refId = ref && (await page.locator('aria-ref=' + ref).getAttribute('id'))
  return {
    ai,
    refId,
    plain: await page.locator('body').ariaSnapshot(),
    page: await page.ariaSnapshot({ mode: 'ai' }),
    first: await page.locator('body').first().ariaSnapshot(),
  }
`

test('ariaSnapshot of body answers the page alone, with or without the takeover bar', {
  skip,
  timeout: 300_000,
}, async (t) => {
  const { playwright, hidden, shown } = await compareWithBar(t, { source })
  assert.ok(playwright.value, JSON.stringify(playwright))
  assert.match(playwright.value.ai, /textbox "Inner name"/)
  assert.equal(playwright.value.refId, 'inner-name')
  for (const [bar, neo] of Object.entries({ hidden, shown })) {
    assert.deepEqual(
      neo.value,
      playwright.value,
      `bar ${bar}: ${JSON.stringify(neo.error)}`,
    )
    assert.doesNotMatch(
      JSON.stringify(neo.value),
      /Hand back|BrowserOS neo|document/,
    )
  }
})
