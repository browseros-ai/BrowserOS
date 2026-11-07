/**
 * The evaluation chain's realm half without a browser: what `serializeArgument` sends (page
 * format, handle guids in reference order), what `parseResult` makes of page JSON, Playwright's
 * client refusals, `normalizeEvaluationExpression`, and the base64 stand-ins `vendor-shims.ts`
 * gives the vendored serializers (checked against Bun's own `atob`/`btoa`). The parity files
 * (`tests/parity/evaluate*.test.mjs`) run the same chain through a real page.
 */

import { expect, test } from 'bun:test'
import { Connection } from '../../src/connection'
import {
  evaluateParams,
  JSHandle,
  normalizeEvaluationExpression,
  parseResult,
  serializeArgument,
} from '../../src/js-handle'
import { atob, btoa, URL } from '../../src/vendor-shims'

const handle = (guid: string) =>
  new JSHandle(new Connection(0), guid, { preview: 'JSHandle@object' })

test('serializeArgument writes page format and lists handles in reference order', () => {
  const a = handle('handle@x.1')
  const b = handle('handle@x.2')
  expect(serializeArgument(5)).toEqual({ value: 5, handles: [] })
  expect(serializeArgument(undefined)).toEqual({
    value: { v: 'undefined' },
    handles: [],
  })
  expect(serializeArgument(-0)).toEqual({ value: { v: '-0' }, handles: [] })
  expect(serializeArgument({ b, list: [a, b] })).toEqual({
    value: {
      o: [
        { k: 'b', v: { h: 0 } },
        { k: 'list', v: { a: [{ h: 1 }, { h: 2 }], id: 2 } },
      ],
      id: 1,
    },
    handles: ['handle@x.2', 'handle@x.1', 'handle@x.2'],
  })
})

test('serializeArgument keeps a circular argument as references', () => {
  const circular: { name: string; self?: unknown } = { name: 'c' }
  circular.self = circular
  expect(serializeArgument(circular)).toEqual({
    value: {
      o: [
        { k: 'name', v: 'c' },
        { k: 'self', v: { ref: 1 } },
      ],
      id: 1,
    },
    handles: [],
  })
})

test('a function argument is refused as Playwright refuses it', () => {
  expect(() => serializeArgument({ a: { inner: () => {} } })).toThrow(
    /Attempting to serialize unexpected value at position "a.inner"/,
  )
  expect(() =>
    evaluateParams('page.evaluate', () => 1, undefined, {
      exposeFunctions: true,
    }),
  ).toThrow(/exposeFunctions/)
})

test('parseResult reads page JSON as Playwright reads it', () => {
  expect(parseResult(undefined)).toBeUndefined()
  expect(Object.is(parseResult({ v: '-0' }), -0)).toBe(true)
  expect(parseResult({ bi: '42' })).toBe(42n)
  expect(parseResult({ d: '2020-05-27T01:31:38.506Z' })).toEqual(
    new Date('2020-05-27T01:31:38.506Z'),
  )
  const circular = parseResult({ o: [{ k: 'b', v: { ref: 1 } }], id: 1 }) as {
    b: unknown
  }
  expect(circular.b).toBe(circular)
  expect(parseResult('ref: <Window>')).toBe('ref: <Window>')
})

test('normalizeEvaluationExpression wraps functions and method shorthands', () => {
  expect(normalizeEvaluationExpression('() => 1', true)).toBe('() => 1')
  expect(normalizeEvaluationExpression('function f() {}', true)).toBe(
    '(function f() {})',
  )
  expect(normalizeEvaluationExpression('sum(a) { return a }', true)).toBe(
    '(function sum(a) { return a })',
  )
  expect(
    normalizeEvaluationExpression('async mult(a) { return a }', true),
  ).toBe('(async function mult(a) { return a })')
  expect(() => normalizeEvaluationExpression('[native code]', true)).toThrow(
    'Passed function is not well-serializable!',
  )
  expect(normalizeEvaluationExpression(' 1 + 2 ', false)).toBe('1 + 2')
})

test('the base64 stand-ins agree with the platform for every byte', () => {
  const all = String.fromCharCode(...Array.from({ length: 256 }, (_, i) => i))
  for (const binary of ['', 'a', 'ab', 'abc', 'abcd', all, all.slice(7, 200)]) {
    expect(btoa(binary)).toBe(globalThis.btoa(binary))
    expect(atob(btoa(binary))).toBe(binary)
    expect(atob(globalThis.btoa(binary))).toBe(binary)
  }
  expect(() => btoa('Ā')).toThrow()
})

test('the URL stand-in keeps the text Playwright returns', () => {
  const url = new URL('https://a.test/x?y=1#z')
  expect(String(url)).toBe('https://a.test/x?y=1#z')
  expect(url.href).toBe('https://a.test/x?y=1#z')
  expect(JSON.stringify({ url })).toBe('{"url":"https://a.test/x?y=1#z"}')
  expect(Object.prototype.toString.call(url)).toBe('[object URL]')
})
