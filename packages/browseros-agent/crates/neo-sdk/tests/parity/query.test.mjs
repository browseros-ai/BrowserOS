// page.$ / page.$$ / frame.$ / ElementHandle.$ / ElementHandle.$$ through the Playwright
// reference and through Neo on fresh query.html pages: which elements match (by their handles'
// previews and a main-world read), null or [] for none, a scope that keeps matches inside its
// element, and strictness where Playwright applies it. Values by `describe` (values.mjs), errors
// by category; node previews once settled (values.mjs).
//
// Cases from tests/page/ at 26137face: queryselector.spec.ts QS-002 "should query existing element
// with css selector", QS-003 "with text selector", QS-004 "with xpath selector", QS-005 "should
// return null for non-existing element", QS-011 "should support >> syntax", QS-012 "should query
// existing elements", QS-013 "should return empty array if nothing is found";
// elementhandle-query-selector.spec.ts EH-067 "should query existing element", EH-068 "should
// return null for non-existing element", EH-070 "should query existing elements", EH-071 "should
// return empty array for non-existing elements".
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'
import { SETTLED, VALUES } from './values.mjs'

const html = readFileSync(
  new URL('../pages/query.html', import.meta.url),
  'utf8',
)

test('page and frame queries find what Playwright finds', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `${VALUES}${SETTLED}
      return {
        css: await outcome(async () => settled(await page.$('section'))),
        text: await outcome(async () => settled(await page.$('text="in-2"'))),
        xpath: await outcome(async () => settled(await page.$('xpath=/html/body/section'))),
        missing: await outcome(async () => page.$('non-existing-element')),
        chained: await outcome(async () => settled(await page.$('#outer >> .inner'))),
        all: await outcome(async () => previews(await page.$$('.inner'))),
        allNone: await outcome(async () => page.$$('non-existing-element')),
        readThrough: await outcome(async () => page.evaluate((e) => e.id, await page.$('section'))),
        frameForm: await outcome(async () => settled(await page.mainFrame().$('#list li'))),
        strictTwo: await outcome(() => page.$('.inner', { strict: true })),
        invalid: await outcome(() => page.$('#outer >> ]')),
      }
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(outcome.value.missing, { value: null })
  assert.deepEqual(outcome.value.allNone, { value: [] })
  assert.equal(outcome.value.strictTwo.error, 'Error:strict')
})

test('element handle queries stay inside their element', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `${VALUES}${SETTLED}
      const outer = await page.$('#outer')
      return {
        one: await outcome(async () => settled(await outer.$('.inner'))),
        none: await outcome(async () => outer.$('section')),
        all: await outcome(async () => previews(await outer.$$('.inner'))),
        allNone: await outcome(async () => outer.$$('li')),
        nested: await outcome(async () => (await outer.$$('*')).length),
      }
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(outcome.value.all.value, [
    'JSHandle@<div class="inner">in-1</div>',
    'JSHandle@<div class="inner">in-2</div>',
  ])
})
