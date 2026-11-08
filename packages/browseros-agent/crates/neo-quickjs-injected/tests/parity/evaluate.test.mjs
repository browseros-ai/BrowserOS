// page.evaluate through the Playwright reference and through Neo on fresh evaluate.html pages:
// what crosses each way (Playwright's serializers on both sides), what a page throws, and what
// the client refuses before sending. Values are compared as `describe` reports them
// (values.mjs); errors by category and by the words Playwright's own tests look for.
//
// Cases from tests/page/page-evaluate.spec.ts at 26137face: EV-001 "should work", EV-002..005
// "should transfer NaN / -0 / Infinity / -Infinity", EV-006 "should roundtrip unserializable
// values", EV-011 "should transfer typed arrays", EV-012 "should transfer bigint", EV-013 "should
// transfer maps as empty objects", EV-022 "should throw when evaluation triggers reload", EV-023
// "should await promise", EV-027 "should reject promise with exception", EV-030 "should reject
// when a falsy value is thrown", EV-031 "should include a non-error rejection value in the error
// message", EV-038 "should throw when passed more than one parameter", EV-040/042/043 (undefined
// and null arguments and fields), EV-050 "should throw usable message for unserializable shallow
// function", EV-054 "should alias Window, Document and Node", EV-055 "should work for circular
// object", EV-057/059 (expression strings), and the Date round trip at the end of the file.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'
import { VALUES } from './values.mjs'

const html = readFileSync(
  new URL('../pages/evaluate.html', import.meta.url),
  'utf8',
)

test('page.evaluate transfers values both ways as Playwright does', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `${VALUES}
      const circular = { name: 'c' }
      circular.self = circular
      const shared = { x: 1 }
      return {
        basic: await outcome(() => page.evaluate(() => 7 * 3)),
        nan: await outcome(() => page.evaluate((a) => a, NaN)),
        negativeZero: await outcome(() => page.evaluate((a) => a, -0)),
        infinity: await outcome(() => page.evaluate((a) => a, Infinity)),
        negativeInfinity: await outcome(() => page.evaluate((a) => a, -Infinity)),
        unserializable: await outcome(() => page.evaluate((v) => v, { infinity: Infinity, nInfinity: -Infinity, nZero: -0, nan: NaN })),
        undefinedArgument: await outcome(() => page.evaluate((x) => ({ a: x }), undefined)),
        undefinedField: await outcome(() => page.evaluate(() => ({ a: undefined }))),
        nullArgument: await outcome(() => page.evaluate((x) => x === null, null)),
        nullResult: await outcome(() => page.evaluate(() => null)),
        undefinedResult: await outcome(() => page.evaluate(() => undefined)),
        date: await outcome(() => page.evaluate(() => ({ date: new Date('2020-05-27T01:31:38.506Z') }))),
        dateArgument: await outcome(() => page.evaluate((d) => d instanceof Date && d.toISOString(), new Date('2020-05-27T01:31:38.506Z'))),
        regexp: await outcome(() => page.evaluate(() => /fo+o/gi)),
        regexpArgument: await outcome(() => page.evaluate((r) => r instanceof RegExp && r.test('FOO'), /fo+/i)),
        url: await outcome(() => page.evaluate(() => new URL('https://a.test/x?y=1#z'))),
        mapResult: await outcome(() => page.evaluate(() => new Map([[1, 2]]))),
        mapArgument: await outcome(() => page.evaluate((a) => a.x.constructor.name + ' ' + JSON.stringify(a.x), { x: new Map([[1, 2]]) })),
        setResult: await outcome(() => page.evaluate(() => new Set([1, 2]))),
        bigint: await outcome(() => page.evaluate(() => 42n)),
        bigintArgument: await outcome(() => page.evaluate((a) => a, 17n)),
        typed: await outcome(() => page.evaluate((a) => a, new Float64Array([1.1, 2.2, 3.3]))),
        typedBigInt: await outcome(() => page.evaluate((a) => a, new BigInt64Array([1n, 2n, 3n]))),
        typedResult: await outcome(() => page.evaluate(() => new Uint8Array([0, 128, 255]))),
        arrays: await outcome(() => page.evaluate((a) => Array.isArray(a) && a, [1, 2, 3])),
        circularResult: await outcome(() => page.evaluate(() => { const a = {}; a.b = a; return a })),
        circularArgument: await outcome(() => page.evaluate((c) => c.self === c && c.name, circular)),
        sharedArgument: await outcome(() => page.evaluate((x) => x, { foo: shared, bar: [shared], baz: { foo: shared } })),
        promise: await outcome(() => page.evaluate(() => Promise.resolve(8 * 7))),
        promiseOfUnserializable: await outcome(() => page.evaluate((v) => Promise.resolve(v), -0)),
        expression: await outcome(() => page.evaluate('1 + 2')),
        expressionWithComments: await outcome(() => page.evaluate('2 + 5;\\n// do some math!')),
        shorthand: await outcome(() => page.evaluate({ sum(a) { return a[0] + a[1] } }.sum, [1, 2])),
        asyncShorthand: await outcome(() => page.evaluate({ async mult(a) { return a[0] * a[1] } }.mult, [2, 3])),
        pageGlobal: await outcome(() => page.evaluate(() => window.__probe)),
        aliases: await outcome(() => page.evaluate('[window, document, document.body]')),
        mainFrame: await outcome(() => page.mainFrame().evaluate(() => document.title)),
      }
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(outcome.value.basic, { value: 21 })
  assert.deepEqual(outcome.value.negativeZero, { value: { number: '-0' } })
  assert.deepEqual(outcome.value.url, {
    value: { url: 'https://a.test/x?y=1#z', href: 'https://a.test/x?y=1#z' },
  })
  assert.deepEqual(outcome.value.mapArgument, { value: 'Object {}' })
})

test('page.evaluate rejects what Playwright rejects, by category', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `${VALUES}
      const results = {
        missingName: await outcome(() => page.evaluate(() => not_existing_object.property), ['not_existing_object']),
        thrownString: await outcome(() => page.evaluate(() => { throw 'qwerty' }), ['qwerty']),
        thrownNull: await outcome(() => page.evaluate(() => { throw null }), ['null']),
        thrownZero: await outcome(() => page.evaluate('(() => { throw 0; })()')),
        rejectedNumber: await outcome(() => page.evaluate(() => Promise.reject(7)), ['7']),
        rejectedFalse: await outcome(() => page.evaluate('Promise.reject(false)')),
        functionArgument: await outcome(() => page.evaluate((arg) => arg, () => {}), ['Attempting to serialize unexpected value']),
        nestedFunctionArgument: await outcome(() => page.evaluate((arg) => arg, { a: { inner: { property: () => {} } } }), ['a.inner.property']),
        tooManyArguments: await outcome(() => page.evaluate((a, b) => false, 1, 2), ['Too many arguments']),
        badOptions: await outcome(() => page.evaluate(() => 1, undefined, 5), ['Too many arguments']),
      }
      results.afterErrors = await outcome(() => page.evaluate(() => 'still fine'))
      // Last: the page reloads under the call (EV-022).
      results.reload = await outcome(() => page.evaluate(() => { location.reload(); return new Promise(() => {}) }), ['navigation'])
      // Playwright's very next evaluate can still land in the dying document (it answers
      // "Execution context was destroyed"); Neo's waits for the new one. That race is not
      // compared: both wait until the reloaded page answers, so the probe reads a settled page.
      for (let attempt = 0; attempt < 100; attempt++) {
        const ready = await page.evaluate(() => document.readyState === 'complete').catch(() => false)
        if (ready) break
      }
      return results
    `,
    probe: 'window.__facts()',
  })
  assert.deepEqual(outcome.value.missingName, {
    error: 'Error',
    mentions: ['not_existing_object'],
  })
  assert.deepEqual(outcome.value.reload, {
    error: 'Error',
    mentions: ['navigation'],
  })
})
