// innerText through the Playwright reference and through Neo on fresh element-text pages: the
// rendered text (not textContent) on each receiver, a hidden element's text, and rejections by
// category only: a non-HTML (SVG) element, a strict locator with two matches, a missing element,
// a selector that is not a string. The short timeouts turn a retried refusal into a timeout.
//
// Ids, not tag selectors: BrowserOS neo's takeover bar adds an open shadow root with buttons
// and SVGs to every page, and Playwright's CSS engine pierces it.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

test('innerText returns rendered text and rejects non-HTML, ambiguous or missing targets', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/element-text.html', import.meta.url),
      'utf8',
    ),
    source: `
      const values = {
        page: await page.innerText('#mixed', null),
        frame: await page.mainFrame().innerText('#list > li', null),
        locator: await page.locator('#hidden').innerText(),
      }
      const textContent = await page.textContent('#mixed')
      const errors = {}
      for (const [name, read] of Object.entries({
        svg: () => page.locator('#label').innerText({ timeout: 1000 }),
        strict: () => page.locator('#list > li').innerText({ timeout: 1000 }),
        missing: () => page.locator('#missing').innerText({ timeout: 500 }),
        notAString: () => page.innerText(42),
      })) {
        try { await read(); errors[name] = 'resolved' } catch (error) { errors[name] = errorCategory(error) }
      }
      return { values, types: Object.values(values).map(v => typeof v), textContent, errors }
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(outcome.value, {
    values: {
      page: 'Hello LOUD\nnext line',
      frame: 'First',
      locator: 'Hidden text',
    },
    types: ['string', 'string', 'string'],
    textContent: 'Hello loudnext line secret',
    errors: {
      svg: 'error',
      strict: 'strict',
      missing: 'timeout',
      notAString: 'error',
    },
  })
  assert.equal(outcome.calls.length, 8)
  assert.ok(outcome.calls.every((c) => c.delivery === undefined))
})
