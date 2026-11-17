/** Neo: Wire values must preserve distinctions JavaScript and UTF-8 JSON normally erase. */

import assert from 'node:assert/strict'
import test from 'node:test'
import { codecFactory } from './codec.mjs'

const codec = codecFactory({
  identify: () => undefined,
  resolve: () => {
    throw new Error('foreign reference')
  },
})
const roundtrip = (value) =>
  codec.decode(JSON.parse(JSON.stringify(codec.encode(value))))

test('special values, UTF-16 keys and regexp state survive ASCII-safe frames', () => {
  const regexp = /a\ud800/gi
  regexp.lastIndex = 3
  const record = {
    '\ud800': '\udc00',
    replacement: '\ufffd',
    regexp,
    big: 999999999999999999999999n,
    nan: NaN,
    negativeZero: -0,
    positive: Infinity,
    negative: -Infinity,
    missing: undefined,
    empty: null,
  }
  const result = roundtrip(record)
  assert.deepEqual(result, record)
  assert.equal(result.regexp.lastIndex, 3)
  assert.equal(result['\ud800'].charCodeAt(0), 0xdc00)
  assert.notDeepEqual(codec.encode('\ud800'), codec.encode('\ufffd'))
  assert.notDeepEqual(codec.encode(undefined), codec.encode(null))
})

test('holes, undefined entries and own __proto__ cannot collapse or mutate the decoder', () => {
  const array = new Array(3)
  array[1] = undefined
  array[2] = null
  const result = roundtrip(array)
  assert.equal(0 in result, false)
  assert.equal(1 in result, true)
  assert.equal(result[1], undefined)
  const record = JSON.parse('{"__proto__":{"polluted":true}}')
  const decoded = roundtrip(record)
  assert.equal(Object.getPrototypeOf(decoded), Object.prototype)
  assert.equal(Object.hasOwn(decoded, '__proto__'), true)
  assert.equal({}.polluted, undefined)
})

test('function source survives exactly without executing in the harness realm', () => {
  class Methods {
    method(value) {
      return value
    }
    async methodAsync(value) {
      return value
    }
  }
  const methods = {
    shorthand(value) {
      return value
    },
    async shorthandAsync(value) {
      return value
    },
  }
  const functions = [
    (value) => value + 1,
    async (value) => value + 1,
    function ordinary(value) {
      return value
    },
    methods.shorthand,
    methods.shorthandAsync,
    Methods.prototype.method,
    Methods.prototype.methodAsync,
    function unicode() {
      return '你好 🦊 \ud800'
    },
    // Include a literal unpaired surrogate, not just an ASCII escape in the source.
    Function('return "\ud800"'),
    () => {
      throw new Error('user code must never run here')
    },
  ]
  for (const fn of functions) {
    const source = Function.prototype.toString.call(fn)
    fn.toString = () => 'overridden'
    const encoded = codec.encode(fn)
    assert.equal(encoded.tag, 'function')
    assert.deepEqual(encoded.sourceUnits, codec.units(source))
    const rebuilt = roundtrip(fn)
    assert.equal(typeof rebuilt, 'function')
    assert.equal(String(rebuilt), source)
    assert.throws(() => rebuilt(), /cannot execute in the harness realm/)
    assert.throws(() => new rebuilt(), /cannot execute in the harness realm/)
  }
  assert.throws(
    () => codec.decode({ tag: 'function', sourceUnits: [65536] }),
    /UTF-16/,
  )
})

test('return-value codecs reject functions instead of inventing callable return values', () => {
  const returns = codecFactory({
    identify: () => undefined,
    allowFunctions: false,
  })
  assert.throws(
    () => returns.encode({ nested: () => {} }),
    /Untransportable function/,
  )
})

test('symbols, cycles, custom objects and foreign refs fail visibly', () => {
  const cycle = {}
  cycle.self = cycle
  for (const value of [Symbol('x'), cycle, new Date()])
    assert.throws(() => codec.encode(value))
  assert.throws(
    () =>
      codec.decode({
        tag: 'object',
        ref: { epoch: 'other', id: 1, family: 'Page' },
      }),
    /foreign/,
  )
  assert.throws(() => codec.decode({ tag: 'string', units: [65536] }), /UTF-16/)
})
