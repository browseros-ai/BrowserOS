// locator.evaluateHandle through the Playwright reference and through Neo on fresh query.html
// pages: the function runs on the one element the locator matches (waiting for it to be
// attached), and the answer is a handle (an element handle for a node); more than one match is a
// strict mode violation, and a missing element waits out the timeout. Values by `describe`
// (values.mjs), errors by category.
//
// Playwright's `Locator.evaluateHandle` (client/locator.ts) is `_withElement(h =>
// h.evaluateHandle(pageFunction, arg, options))`; its locator-evaluate.spec.ts cases LO-084 "should
// work" and LO-085 "should retrieve content from subtree" are run here through the handle form.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'
import { SETTLED, VALUES } from './values.mjs'

test('locator.evaluateHandle answers a handle to the result', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(new URL('../pages/query.html', import.meta.url), 'utf8'),
    source: `${VALUES}${SETTLED}
      return {
        value: await outcome(async () => (await page.locator('.tweet .like').evaluateHandle((node) => node.innerText)).jsonValue()),
        subtree: await outcome(async () => (await page.locator('#outer .inner >> nth=1').evaluateHandle((node) => ({ text: node.textContent }))).jsonValue()),
        object: await outcome(async () => (await page.locator('section').evaluateHandle((node) => node.dataset)).toString()),
        element: await outcome(async () => {
          const handle = await page.locator('#list').evaluateHandle((node) => node.firstElementChild)
          return { isElement: handle.asElement() === handle, preview: await settled(handle) }
        }),
        argument: await outcome(async () => (await page.locator('section').evaluateHandle((node, suffix) => node.id + suffix, '?')).jsonValue()),
        strict: await outcome(() => page.locator('.inner').evaluateHandle((node) => node)),
        missing: await outcome(() => page.locator('#missing').evaluateHandle((node) => node, undefined, { timeout: 300 })),
      }
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(outcome.value.value, { value: '100' })
  assert.equal(outcome.value.strict.error, 'Error:strict')
  assert.equal(outcome.value.missing.error, 'TimeoutError')
})
