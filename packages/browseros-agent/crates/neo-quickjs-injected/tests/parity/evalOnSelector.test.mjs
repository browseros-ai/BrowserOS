// page.$eval, frame.$eval, ElementHandle.$eval and ElementHandle.$$eval through the Playwright
// reference and through Neo on fresh query.html pages: the function runs on the first match (or,
// for an element handle's `$$eval`, on the array of that element's matches) in the page's main
// world, with an argument (a value or an element handle); no match fails `$eval` and gives
// `$$eval` an empty array. Values by `describe` (values.mjs), errors by category and by the words
// Playwright's own tests look for. The page's and frame's `$$eval` are in evaluateAll.test.mjs.
//
// Cases from tests/page/ at 26137face: eval-on-selector.spec.ts ES-001 "should work with css
// selector", ES-008 "should work with text selector", ES-012 "should accept arguments", ES-013
// "should accept ElementHandles as arguments", ES-014 "should throw error if no element is found",
// ES-015 "should support >> syntax"; elementhandle-eval-on-selector.spec.ts EH-039 "should work",
// EH-041 "should throw in case of missing selector", EH-042 "should work for all", EH-044 "should
// not throw in case of missing selector for all".
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'
import { VALUES } from './values.mjs'

const html = readFileSync(
  new URL('../pages/query.html', import.meta.url),
  'utf8',
)

test('$eval runs on the first match as Playwright runs it', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `${VALUES}
      const outer = await page.$('#outer')
      return {
        css: await outcome(() => page.$eval('section', (e) => e.id)),
        text: await outcome(() => page.$eval('text="in-1"', (e) => e.textContent)),
        argument: await outcome(() => page.$eval('section', (e, suffix) => e.id + suffix, '!')),
        elementArgument: await outcome(() => page.$eval('section', (e, other) => e.id + ':' + other.id, outer)),
        chained: await outcome(() => page.$eval('#outer >> .inner', (e) => e.textContent)),
        missing: await outcome(() => page.$eval('#missing', (e) => e.id), ['Failed to find element matching selector']),
        writes: await outcome(() => page.$eval('section', (e) => { window.__touched = e.id; return true })),
        frameForm: await outcome(() => page.mainFrame().$eval('#list li', (e) => e.textContent)),
        tooMany: await outcome(() => page.$eval('section', (a, b) => 1, 1, 2), ['Too many arguments']),
      }
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(outcome.value.missing, {
    error: 'Error',
    mentions: ['Failed to find element matching selector'],
  })
  assert.deepEqual(outcome.value.elementArgument, { value: 'greeting:outer' })
})

test('element handle $eval and $$eval run inside their element', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `${VALUES}
      const tweet = await page.$('.tweet')
      return {
        one: await outcome(() => tweet.$eval('.like', (node) => node.innerText)),
        oneMissing: await outcome(() => tweet.$eval('.a', (node) => node.innerText), ['Failed to find element matching selector']),
        oneArgument: await outcome(() => tweet.$eval('.retweets', (node, n) => Number(node.innerText) + n, 5)),
        all: await outcome(() => tweet.$$eval('div', (nodes) => nodes.map((n) => n.innerText))),
        allMissing: await outcome(() => tweet.$$eval('.a', (nodes) => nodes.length)),
      }
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(outcome.value.all, { value: ['100', '10'] })
  assert.deepEqual(outcome.value.oneArgument, { value: 15 })
})
