// locator.elementHandle and locator.elementHandles through the Playwright reference and through
// Neo on fresh query.html pages. Playwright's `Locator.elementHandle` waits for the one element
// the locator matches (`waitForSelector` with `strict: true, state: 'attached'`), and
// `elementHandles` is `this._frame.$$(this._selector)`, every match without waiting
// (client/locator.ts); each answer is an element handle the page can read. Values by `describe`
// (values.mjs), errors by category; node previews once settled (values.mjs).
//
// Cases from tests/page/locator-element-handle.spec.ts at 26137face: LO-079 "should query existing
// element", LO-080 "should query existing elements", LO-081 "should return empty array for
// non-existing elements", LO-082 "xpath should query existing element".
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'
import { SETTLED, VALUES } from './values.mjs'

test("locator.elementHandle and elementHandles answer the locator's elements", {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(new URL('../pages/query.html', import.meta.url), 'utf8'),
    source: `${VALUES}${SETTLED}
      const read = (handle) => page.evaluate((e) => e.textContent, handle)
      return {
        one: await outcome(async () => settled(await page.locator('#outer .inner >> nth=1').elementHandle())),
        chained: await outcome(async () => read(await page.locator('html').locator('#outer').locator('.inner').first().elementHandle())),
        xpath: await outcome(async () => read(await page.locator('html').locator('xpath=./body/ul').locator('xpath=./li[2]').elementHandle())),
        isElement: await outcome(async () => { const h = await page.locator('section').elementHandle(); return h.asElement() === h }),
        all: await outcome(async () => previews(await page.locator('#list li').elementHandles())),
        allRead: await outcome(async () => { const out = []; for (const h of await page.locator('html').locator('li').elementHandles()) out.push(await read(h)); return out }),
        allNone: await outcome(async () => page.locator('html').locator('#missing').elementHandles()),
        strict: await outcome(() => page.locator('.inner').elementHandle({ timeout: 1000 })),
        missing: await outcome(() => page.locator('#missing').elementHandle({ timeout: 300 })),
      }
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(outcome.value.allRead, { value: ['one', 'two', 'three'] })
  assert.deepEqual(outcome.value.allNone, { value: [] })
  assert.equal(outcome.value.strict.error, 'Error:strict')
  assert.equal(outcome.value.missing.error, 'TimeoutError')
})
