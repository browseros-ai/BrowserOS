// page.evaluateHandle and JSHandle.evaluate/evaluateHandle/toString/asElement through the
// Playwright reference and through Neo on fresh evaluate.html pages: a handle passed back as an
// argument (alone, nested, repeated, to a primitive, to an unserializable value), a handle's own
// evaluations (function and expression), its preview, and which handles are elements.
//
// Cases from tests/page/ at 26137face: page-evaluate-handle.spec.ts EV-101 "should work", EV-102
// "should accept object handle as an argument", EV-103 "should accept object handle to primitive
// types", EV-104 "should accept nested handle", EV-105 "should accept nested window handle",
// EV-106 "should accept multiple nested handles", EV-107 "should accept same handle multiple
// times", EV-109 "should accept object handle to unserializable value", EV-111 "should work with
// primitives"; jshandle-evaluate.spec.ts JH-005 "should work with function", JH-006 "should work
// with expression"; jshandle-to-string.spec.ts JH-018 "should work for primitives", JH-019
// "should work for complicated objects", JH-022 "should work with different subtypes", JH-023
// "should work with previewable subtypes"; jshandle-as-element.spec.ts JH-001 "should work",
// JH-002 "should return null for non-elements", JH-003 "should return ElementHandle for
// TextNodes", JH-004 "should work with nullified Node"; page-evaluate.spec.ts EV-060 "should
// accept element handle as an argument". A node handle's preview is compared once settled: Neo's
// is final when the handle is created, where Playwright's starts as `JSHandle@node`.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'
import { VALUES } from './values.mjs'

const html = readFileSync(
  new URL('../pages/evaluate.html', import.meta.url),
  'utf8',
)

test('evaluateHandle handles cross back as arguments and evaluate on their own', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `${VALUES}
      const windowHandle = await page.evaluateHandle(() => window)
      const navigatorHandle = await page.evaluateHandle(() => navigator)
      const five = await page.evaluateHandle(() => 5)
      const object = await page.evaluateHandle(() => ({ x: 1, y: 'foo' }))
      const list = await page.evaluateHandle(() => ['baz'])
      const infinity = await page.evaluateHandle(() => Infinity)
      const one = await page.evaluateHandle(() => 1)
      const fooWindow = await page.evaluateHandle(() => { window.foo = [1, 2]; window.FOO = 123; return window })
      const section = await page.evaluateHandle(() => document.querySelector('#s'))
      return {
        windowIsObject: typeof windowHandle === 'object' && windowHandle !== null,
        navigatorArgument: await outcome(() => page.evaluate((n) => n.userAgent.includes('Mozilla'), navigatorHandle)),
        primitiveArgument: await outcome(() => page.evaluate((e) => Object.is(e, 5), five)),
        nested: await outcome(() => page.evaluate(({ foo }) => foo, { foo: object })),
        nestedWindow: await outcome(() => page.evaluate(({ foo }) => foo === window, { foo: windowHandle })),
        multipleNested: await outcome(() => page.evaluate((x) => JSON.stringify(x), { a1: { foo: object }, a2: { bar: five, arr: [{ baz: list }] } })),
        sameHandleTwice: await outcome(() => page.evaluate((x) => x, { foo: one, bar: [one], baz: { foo: one } })),
        unserializable: await outcome(() => page.evaluate((e) => Object.is(e, Infinity), infinity)),
        windowGlobal: await outcome(() => page.evaluate((e) => e.FOO, fooWindow)),
        elementArgument: await outcome(() => page.evaluate((e) => e.textContent, section)),
        handleFunction: await outcome(() => fooWindow.evaluate((w) => w.foo)),
        handleExpression: await outcome(() => fooWindow.evaluate('window.foo')),
        handleWithArgument: await outcome(() => object.evaluate((o, suffix) => o.y + suffix, '!')),
        handleOfHandle: await outcome(async () => (await object.evaluateHandle((o) => o.y)).jsonValue()),
        primitiveReceiver: await outcome(() => five.evaluate((n, m) => n * m, 3)),
        frameEvaluateHandle: await outcome(async () => (await page.mainFrame().evaluateHandle(() => document.title)).jsonValue()),
      }
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(outcome.value.navigatorArgument, { value: true })
  assert.deepEqual(outcome.value.handleFunction, { value: [1, 2] })
})

test('JSHandle previews and asElement read as Playwright reads them', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `${VALUES}
      // Playwright fills a node's preview in by event after creation (JH-022 polls for it); Neo's
      // is final at once (a recorded Deviation). Both are read once settled.
      const settled = async (handle) => {
        for (let attempt = 0; attempt < 100 && handle.toString() === 'JSHandle@node'; attempt++)
          await new Promise((resolve) => setTimeout(resolve, 20))
        return handle.toString()
      }
      const preview = async (expression) => settled(await page.evaluateHandle(expression))
      const previews = {
        number: await preview(() => 2),
        string: await preview(() => 'a'),
        window: await preview(() => window),
        twelve: await preview('12'),
        true: await preview('true'),
        undefined: await preview('undefined'),
        foo: await preview('"foo"'),
        symbol: await preview('Symbol()'),
        null: await preview('null'),
        negativeZero: await preview('-0'),
        nan: await preview('NaN'),
        weakMap: await preview('new WeakMap()'),
        weakSet: await preview('new WeakSet()'),
        proxy: await preview('new Proxy({}, {})'),
        regexp: await preview('/foo/'),
        object: await preview(() => ({ a: 1 })),
        section: await preview(() => document.querySelector('#s')),
        para: await preview(() => document.querySelector('#para')),
        textNode: await preview(() => document.querySelector('#text').firstChild),
      }
      const contains = {
        function: (await preview('(function(){})')).includes('function'),
        map: (await preview('new Map()')).includes('Map'),
        set: (await preview('new Set()')).includes('Set'),
        array: (await preview('[]')).includes('Array'),
        error: (await preview('new Error()')).includes('Error'),
        date: (await preview('new Date(0)')).includes('GMT'),
        int32: (await preview('new Int32Array()')).includes('Int32Array'),
      }
      const body = await page.evaluateHandle(() => document.body)
      const two = await page.evaluateHandle(() => 2)
      const text = await page.evaluateHandle(() => document.querySelector('#text').firstChild)
      const asElement = {
        body: body.asElement() === body,
        primitive: two.asElement(),
        textNode: text.asElement() === text,
        textNodeType: await page.evaluate((e) => e.nodeType === Node.TEXT_NODE, text.asElement()),
      }
      await page.evaluate('delete Node')
      const section = await page.evaluateHandle(() => document.querySelector('section'))
      asElement.nullifiedNode = section.asElement() !== null
      return { previews, contains, asElement }
    `,
    probe: 'window.__facts()',
  })
  assert.equal(outcome.value.previews.window, 'Window')
  assert.equal(
    outcome.value.previews.section,
    'JSHandle@<section id="s" data-x="sec">42</section>',
  )
  assert.equal(outcome.value.asElement.primitive, null)
})
