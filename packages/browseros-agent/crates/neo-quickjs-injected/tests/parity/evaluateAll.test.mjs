// locator.evaluateAll, page.$$eval and frame.$$eval through the Playwright reference and through
// Neo on fresh query.html pages. They are one path: Playwright's `Locator.evaluateAll` is
// `this._frame.$$eval(this._selector, pageFunction, arg)` (client/locator.ts). The function runs
// on the array of every match, in document order, in the page's main world, with an argument; no
// match gives it an empty array. Values by `describe` (values.mjs), errors by category, and what
// a function marked on the page by `window.__facts()`.
//
// Cases from tests/page/ at 26137face: eval-on-selector-all.spec.ts ES-027 "should work with css
// selector", ES-031 "should support >> syntax", ES-034 "should return complex values";
// locator-evaluate.spec.ts LO-086 "should work for all", LO-087 "should retrieve content from
// subtree for all", LO-088 "should not throw in case of missing selector for all".
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'
import { VALUES } from './values.mjs'

test('evaluateAll and $$eval run on every match as Playwright runs them', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(new URL('../pages/query.html', import.meta.url), 'utf8'),
    source: `${VALUES}
      return {
        all: await outcome(() => page.$$eval('#list li', (items) => items.map((e) => e.textContent))),
        chained: await outcome(() => page.$$eval('#outer >> .inner', (items) => items.length)),
        complex: await outcome(() => page.$$eval('#list li', (items) => items.map((e) => ({ text: e.textContent, index: [...e.parentNode.children].indexOf(e) })))),
        none: await outcome(() => page.$$eval('#missing', (items) => items.length)),
        argument: await outcome(() => page.$$eval('.inner', (items, prefix) => items.map((e) => prefix + e.textContent), '>')),
        frameForm: await outcome(() => page.mainFrame().$$eval('#list li', (items) => items.length)),
        marks: await outcome(() => page.$$eval('.inner', (items) => { for (const e of items) e.setAttribute('data-mark', ''); return items.length })),
        locator: await outcome(() => page.locator('.tweet .like').evaluateAll((nodes) => nodes.map((n) => n.innerText))),
        locatorSubtree: await outcome(() => page.locator('#outer').locator('.inner').evaluateAll((nodes) => nodes.map((n) => n.textContent))),
        locatorNone: await outcome(() => page.locator('#missing').evaluateAll((nodes) => nodes.length)),
        locatorArgument: await outcome(() => page.locator('#list li').evaluateAll((nodes, n) => nodes.length * n, 10)),
      }
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(outcome.value.none, { value: 0 })
  assert.deepEqual(outcome.value.locatorSubtree, { value: ['in-1', 'in-2'] })
  assert.deepEqual(outcome.value.locatorArgument, { value: 30 })
})
