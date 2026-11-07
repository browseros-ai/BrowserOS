// locator.evaluate through the Playwright reference and through Neo on fresh evaluate.html pages:
// the function runs on the one element the locator matches, in the page's main world (it sees a
// page global), with an argument (a value or an element handle); a second match is a strict mode
// violation, and a missing element waits out the locator's timeout. Values by `describe`
// (values.mjs), errors by category.
//
// Cases from tests/page/locator-evaluate.spec.ts at 26137face: LO-084 "should work", LO-085
// "should retrieve content from subtree"; and Playwright's `Locator._withElement` contract
// (client/locator.ts): strict `waitForSelector` for an attached element under the locator's
// timeout, the task on its handle, then `dispose`.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'
import { VALUES } from './values.mjs'

test('locator.evaluate runs on the one element, in the main world', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/evaluate.html', import.meta.url),
      'utf8',
    ),
    source: `${VALUES}
      const text = await page.evaluateHandle(() => document.querySelector('#text'))
      return {
        works: await outcome(() => page.locator('.tweet .like').evaluate((node) => node.innerText)),
        subtree: await outcome(() => page.locator('#myId .a').evaluate((node) => node.innerText)),
        argument: await outcome(() => page.locator('#s').evaluate((e, suffix) => e.textContent + suffix, '!')),
        elementArgument: await outcome(() => page.locator('#s').evaluate((e, other) => e.textContent + ' ' + other.textContent, text)),
        mainWorld: await outcome(() => page.locator('#s').evaluate(() => window.__probe)),
        writes: await outcome(() => page.locator('#s').evaluate((e) => { window.__touched = e.id; return e.dataset.x })),
        returnsObject: await outcome(() => page.locator('#para').evaluate((e) => ({ tag: e.tagName, kids: e.children.length }))),
        frameLocator: await outcome(() => page.mainFrame().locator('#text').evaluate((e) => e.textContent)),
        strict: await outcome(() => page.locator('#list > li').evaluate((e) => e.textContent)),
        missing: await outcome(() => page.locator('#missing').evaluate((e) => e.id, undefined, { timeout: 500 })),
        thrown: await outcome(() => page.locator('#s').evaluate(() => { throw new Error('boom') }), ['boom']),
        functionArgument: await outcome(() => page.locator('#s').evaluate((e, f) => f, () => {}), ['Attempting to serialize unexpected value']),
      }
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(outcome.value.works, { value: '100' })
  assert.deepEqual(outcome.value.strict, {
    error: 'Error:strict',
    mentions: [],
  })
  assert.deepEqual(outcome.value.missing, {
    error: 'TimeoutError',
    mentions: [],
  })
})
