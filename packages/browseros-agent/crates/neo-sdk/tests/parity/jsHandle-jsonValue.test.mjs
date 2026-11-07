// JSHandle.jsonValue through the Playwright reference and through Neo on fresh evaluate.html
// pages: objects, dates, circular objects, primitives (which Playwright answers from the value it
// kept), unserializable numbers, and the page objects Playwright's page serializer names instead of
// copying (window, a node). Values by `describe` (values.mjs).
//
// Cases from tests/page/jshandle-json-value.spec.ts at 26137face: JH-007 "should work", JH-008
// "should work with dates", JH-009 "should handle circular objects"; and page-evaluate.spec.ts
// EV-054 "should alias Window, Document and Node" for what a handle to one of those reads as.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'
import { VALUES } from './values.mjs'

test('jsonValue reads a handle as Playwright reads it', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/evaluate.html', import.meta.url),
      'utf8',
    ),
    source: `${VALUES}
      const json = async (expression) => outcome(async () => (await page.evaluateHandle(expression)).jsonValue())
      return {
        object: await json(() => ({ foo: 'bar' })),
        date: await json(() => new Date('2017-09-26T00:00:00.000Z')),
        dateToJSON: await outcome(async () => (await (await page.evaluateHandle(() => new Date('2017-09-26T00:00:00.000Z'))).jsonValue()).toJSON()),
        circular: await json('const a = {}; a.b = a; a'),
        nested: await json(() => ({ list: [1, { two: 2 }], when: new Date(0), re: /x/g })),
        number: await json(() => 5),
        string: await json(() => 'text'),
        null: await json(() => null),
        undefined: await json(() => undefined),
        negativeZero: await json(() => -0),
        nan: await json(() => NaN),
        infinity: await json(() => -Infinity),
        bigint: await json(() => 42n),
        window: await json(() => window),
        node: await json(() => document.querySelector('#s')),
        map: await json(() => new Map([[1, 2]])),
      }
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(outcome.value.object, { value: { object: { foo: 'bar' } } })
  assert.deepEqual(outcome.value.dateToJSON, {
    value: '2017-09-26T00:00:00.000Z',
  })
})
