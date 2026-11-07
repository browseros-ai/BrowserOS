// JSHandle.getProperty and JSHandle.getProperties through the Playwright reference and through Neo
// on fresh evaluate.html pages: one property as a handle (own, missing, a primitive's, an
// unserializable value), and every enumerable own property (none for a primitive; inherited
// fields set by a superclass constructor; an HTMLCollection's indices; an element as an element
// handle). Values by `describe` (values.mjs).
//
// Cases from tests/page/jshandle-properties.spec.ts at 26137face: JH-010 "should work", JH-011
// "should work with undefined, null, and empty", JH-012 "should work with unserializable values",
// JH-013 "getProperties should work", JH-014 "getProperties should return empty map for
// non-objects", JH-015 "getProperties should return properties inhertied from super class", JH-016
// "getProperties should return own enumerable properties only", JH-017 "getProperties should work
// with elements" (read through `evaluate`: ElementHandle.textContent is another ledger row).
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'
import { VALUES } from './values.mjs'

test('getProperty and getProperties read handles as Playwright reads them', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/evaluate.html', import.meta.url),
      'utf8',
    ),
    source: `${VALUES}
      const json = async (handle) => handle.jsonValue()
      const object = await page.evaluateHandle(() => ({ one: 1, two: 2, three: 3, undefined: undefined, null: null }))
      const unserializable = await page.evaluateHandle(() => ({ infinity: Infinity, nInfinity: -Infinity, nan: NaN, nzero: -0 }))
      const keys = async (handle) => [...(await handle.getProperties()).keys()]
      return {
        two: await outcome(async () => json(await object.getProperty('two'))),
        undefined: await outcome(async () => json(await object.getProperty('undefined'))),
        null: await outcome(async () => json(await object.getProperty('null'))),
        empty: await outcome(async () => json(await object.getProperty('empty'))),
        emptyPreview: await outcome(async () => (await object.getProperty('empty')).toString()),
        infinity: await outcome(async () => json(await unserializable.getProperty('infinity'))),
        nInfinity: await outcome(async () => json(await unserializable.getProperty('nInfinity'))),
        nan: await outcome(async () => json(await unserializable.getProperty('nan'))),
        nzero: await outcome(async () => json(await unserializable.getProperty('nzero'))),
        primitive: await outcome(async () => json(await (await page.evaluateHandle(() => 'abc')).getProperty('length'))),
        getter: await outcome(async () => json(await (await page.evaluateHandle(() => document.querySelector('#s'))).getProperty('textContent'))),
        all: await outcome(async () => {
          const properties = await (await page.evaluateHandle(() => ({ foo: 'bar' }))).getProperties()
          return [properties.size, await properties.get('foo').jsonValue()]
        }),
        nonObject: await outcome(async () => (await (await page.evaluateHandle(() => 123)).getProperties()).size),
        inherited: await outcome(async () => {
          const handle = await page.evaluateHandle(() => {
            class A { constructor() { this.a = '1' } }
            class B extends A { constructor() { super(); this.b = '2' } }
            return new B()
          })
          const properties = await handle.getProperties()
          return [await properties.get('a').jsonValue(), await properties.get('b').jsonValue()]
        }),
        ownEnumerableOnly: await outcome(async () => keys(await page.evaluateHandle(() => {
          class A { constructor() { this.own = 1; Object.defineProperty(this, 'ownHidden', { value: 2, enumerable: false }) } }
          A.prototype.inherited = 3
          return new A()
        }))),
        collection: await outcome(async () => keys(await page.evaluateHandle(() => document.querySelector('#list').children))),
        elements: await outcome(async () => {
          const body = (await (await page.evaluateHandle(() => ({ body: document.body }))).getProperties()).get('body')
          return [body.asElement() === body, await body.evaluate((e) => e.querySelector('#s').textContent)]
        }),
      }
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(outcome.value.two, { value: 2 })
  assert.deepEqual(outcome.value.ownEnumerableOnly, { value: ['own'] })
  assert.deepEqual(outcome.value.collection, { value: ['0', '1'] })
})
