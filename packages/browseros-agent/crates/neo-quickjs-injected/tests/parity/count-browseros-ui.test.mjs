// count, nth and last on a page carrying BrowserOS's takeover bar (browseros-ui.mjs): Neo on
// BrowserOS as shipped, the bar in place, against Playwright on a browser without it. The bar's
// own <body>, input and buttons never count, the page's own open shadow root still does, and the
// REAL bench's `locator('body').count()` (2 before) is 1. Hidden bar, then shown bar.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareWithBar } from './browseros-ui.mjs'
import { skip } from './harness.mjs'

const source = `
  page.setDefaultTimeout(2000)
  return {
    body: await page.locator('body').count(),
    html: await page.locator('html').count(),
    input: await page.locator('input').count(),
    button: await page.locator('button').count(),
    textbox: await page.getByRole('textbox').count(),
    buttonRole: await page.getByRole('button').count(),
    ownShadow: await page.locator('#own-host button').count(),
    lastInput: await page.locator('input').last().getAttribute('id'),
    secondButton: await page.locator('button').nth(1).textContent(),
    handBack: await page.getByText('Hand back to the agent').count(),
    bodyText: await page.locator('body').innerText(),
  }
`

test("count, nth and last skip the takeover bar and pierce the page's own shadow root", {
  skip,
  timeout: 300_000,
}, async (t) => {
  const { playwright, hidden, shown } = await compareWithBar(t, { source })
  assert.deepEqual(
    { ...playwright.value, bodyText: undefined },
    {
      body: 1,
      html: 1,
      input: 2,
      button: 2,
      textbox: 2,
      buttonRole: 2,
      ownShadow: 1,
      lastInput: 'inner-name',
      secondButton: 'Inner go',
      handBack: 0,
      bodyText: undefined,
    },
    'the reference counts the page alone',
  )
  assert.deepEqual(hidden.value, playwright.value, 'bar hidden')
  // innerText is the layout's text, which includes a shown bar's in any engine (the reference
  // has none); every selector read still skips it.
  assert.deepEqual(
    { ...shown.value, bodyText: undefined },
    { ...playwright.value, bodyText: undefined },
    'bar shown',
  )
  assert.ok(shown.barStill, 'the bar is still shown after the run')
})
