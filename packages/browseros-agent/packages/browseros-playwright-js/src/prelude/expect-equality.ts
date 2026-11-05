/**
 * Jest's equality and printing for the prelude's value matchers: pretty-format's one-line
 * output, the asymmetric matchers (`expect.any`, `expect.objectContaining`, ...) and `equals`,
 * with toEqual's rule that undefined properties do not count and toStrictEqual's that they do.
 */
import { Locator } from './locator'

export function stringify(value: unknown, seen = new Set<unknown>()): string {
  if (typeof value === 'string') return `"${value.replace(/["\\]/g, '\\$&')}"`
  if (typeof value === 'number')
    return Object.is(value, -0) ? '-0' : String(value)
  if (typeof value === 'bigint') return `${value}n`
  if (typeof value === 'function')
    return `[Function ${value.name || 'anonymous'}]`
  if (typeof value !== 'object' || value === null) return String(value)
  if (value instanceof AsymmetricMatcher) return value.toString()
  if (value instanceof RegExp) return String(value)
  if (value instanceof Date)
    return Number.isNaN(+value) ? 'Date { NaN }' : value.toISOString()
  if (value instanceof Error) return `[${value.name}: ${value.message}]`
  if (value instanceof Locator) return `Locator ${value.toString()}`
  if (seen.has(value)) return '[Circular]'
  seen.add(value)
  try {
    if (Array.isArray(value))
      return `[${value.map((item) => stringify(item, seen)).join(', ')}]`
    if (value instanceof Map)
      return `Map {${[...value].map(([key, item]) => `${stringify(key, seen)} => ${stringify(item, seen)}`).join(', ')}}`
    if (value instanceof Set)
      return `Set {${[...value].map((item) => stringify(item, seen)).join(', ')}}`
    const name = Object.getPrototypeOf(value)?.constructor?.name
    const prefix = name && name !== 'Object' ? `${name} ` : ''
    const entries = Object.entries(value).map(
      ([key, item]) => `${JSON.stringify(key)}: ${stringify(item, seen)}`,
    )
    return `${prefix}{${entries.join(', ')}}`
  } finally {
    seen.delete(value)
  }
}

export function receiverText(receiver: unknown): string {
  if (typeof receiver === 'object' && receiver !== null)
    return `${Object.getPrototypeOf(receiver)?.constructor?.name ?? 'Object'} ${stringify(receiver)}`
  return String(receiver)
}

export abstract class AsymmetricMatcher {
  constructor(readonly inverse: boolean) {}
  abstract asymmetricMatch(other: unknown): boolean
  abstract describe(): string
  toString(): string {
    return `${this.inverse ? 'Not' : ''}${this.describe()}`
  }
}

export class Any extends AsymmetricMatcher {
  constructor(private readonly expected: unknown) {
    super(false)
    if (expected === undefined)
      throw new TypeError(
        'any() expects to be passed a constructor function. Please pass one or use anything() to match any object.',
      )
  }
  asymmetricMatch(other: unknown): boolean {
    if (this.expected === String)
      return typeof other === 'string' || other instanceof String
    if (this.expected === Number)
      return typeof other === 'number' || other instanceof Number
    if (this.expected === Boolean)
      return typeof other === 'boolean' || other instanceof Boolean
    if (this.expected === Function) return typeof other === 'function'
    if (this.expected === Symbol) return typeof other === 'symbol'
    if (this.expected === BigInt) return typeof other === 'bigint'
    if (this.expected === Object)
      return typeof other === 'object' && other !== null
    return typeof this.expected === 'function' && other instanceof this.expected
  }
  describe(): string {
    return `Any<${typeof this.expected === 'function' ? this.expected.name : String(this.expected)}>`
  }
}

export class Anything extends AsymmetricMatcher {
  asymmetricMatch(other: unknown): boolean {
    return other !== null && other !== undefined
  }
  describe(): string {
    return 'Anything'
  }
}

export class ArrayContaining extends AsymmetricMatcher {
  constructor(
    private readonly expected: unknown[],
    inverse: boolean,
  ) {
    super(inverse)
  }
  asymmetricMatch(other: unknown): boolean {
    const matches =
      Array.isArray(other) &&
      this.expected.every((item) =>
        other.some((candidate) => equals(candidate, item)),
      )
    return this.inverse ? !matches : matches
  }
  describe(): string {
    return `ArrayContaining ${stringify(this.expected)}`
  }
}

export class ArrayOf extends AsymmetricMatcher {
  constructor(
    private readonly expected: unknown,
    inverse: boolean,
  ) {
    super(inverse)
  }
  asymmetricMatch(other: unknown): boolean {
    const matches =
      Array.isArray(other) && other.every((item) => equals(item, this.expected))
    return this.inverse ? !matches : matches
  }
  describe(): string {
    return `ArrayOf ${stringify(this.expected)}`
  }
}

export class ObjectContaining extends AsymmetricMatcher {
  constructor(
    private readonly expected: Record<string, unknown>,
    inverse: boolean,
  ) {
    super(inverse)
  }
  asymmetricMatch(other: unknown): boolean {
    const matches =
      typeof other === 'object' &&
      other !== null &&
      Object.entries(this.expected).every(
        ([key, item]) => key in other && equals(Reflect.get(other, key), item),
      )
    return this.inverse ? !matches : matches
  }
  describe(): string {
    return `ObjectContaining ${stringify(this.expected)}`
  }
}

export class StringContaining extends AsymmetricMatcher {
  constructor(
    private readonly expected: string,
    inverse: boolean,
  ) {
    super(inverse)
  }
  asymmetricMatch(other: unknown): boolean {
    const matches = typeof other === 'string' && other.includes(this.expected)
    return this.inverse ? !matches : matches
  }
  describe(): string {
    return `StringContaining ${stringify(this.expected)}`
  }
}

export class StringMatching extends AsymmetricMatcher {
  private readonly expected: RegExp
  constructor(expected: string | RegExp, inverse: boolean) {
    super(inverse)
    this.expected =
      typeof expected === 'string' ? new RegExp(expected) : expected
  }
  asymmetricMatch(other: unknown): boolean {
    const matches = typeof other === 'string' && this.expected.test(other)
    return this.inverse ? !matches : matches
  }
  describe(): string {
    return `StringMatching ${String(this.expected)}`
  }
}

export class CloseTo extends AsymmetricMatcher {
  constructor(
    private readonly expected: number,
    private readonly precision: number,
    inverse: boolean,
  ) {
    super(inverse)
  }
  asymmetricMatch(other: unknown): boolean {
    const matches =
      typeof other === 'number' && closeTo(other, this.expected, this.precision)
    return this.inverse ? !matches : matches
  }
  describe(): string {
    return `NumberCloseTo ${this.expected} (${this.precision} digits)`
  }
}

export function closeTo(
  received: number,
  expected: number,
  precision: number,
): boolean {
  if (
    received === Number.POSITIVE_INFINITY &&
    expected === Number.POSITIVE_INFINITY
  )
    return true
  if (
    received === Number.NEGATIVE_INFINITY &&
    expected === Number.NEGATIVE_INFINITY
  )
    return true
  return Math.abs(expected - received) < 10 ** -precision / 2
}

export function equals(
  a: unknown,
  b: unknown,
  strict = false,
  stack: unknown[] = [],
): boolean {
  if (a instanceof AsymmetricMatcher) return a.asymmetricMatch(b)
  if (b instanceof AsymmetricMatcher) return b.asymmetricMatch(a)
  if (a instanceof Error && b instanceof Error) return a.message === b.message
  if (Object.is(a, b)) return true
  if (typeof a !== 'object' || typeof b !== 'object' || !a || !b) return false
  if (Object.prototype.toString.call(a) !== Object.prototype.toString.call(b))
    return false
  if (a instanceof Date || a instanceof RegExp || isBoxed(a))
    return valueEquals(a, b)
  if (strict && Object.getPrototypeOf(a) !== Object.getPrototypeOf(b))
    return false
  // A cycle back to an object already being compared is equal so far, as in Jest.
  if (stack.includes(a)) return true
  return structureEquals(a, b, strict, [...stack, a])
}

function isBoxed(value: object): boolean {
  return (
    value instanceof Number ||
    value instanceof String ||
    value instanceof Boolean
  )
}

/** Dates, RegExps and boxed primitives, which compare by value. */
function valueEquals(a: object, b: object): boolean {
  if (a instanceof Date && b instanceof Date)
    return +a === +b || (Number.isNaN(+a) && Number.isNaN(+b))
  if (a instanceof RegExp && b instanceof RegExp)
    return a.source === b.source && a.flags === b.flags
  return Object.is(a.valueOf(), b.valueOf())
}

function structureEquals(
  a: object,
  b: object,
  strict: boolean,
  stack: unknown[],
): boolean {
  if (a instanceof Map && b instanceof Map)
    return (
      a.size === b.size &&
      [...a].every(
        ([key, value]) =>
          b.has(key) && equals(value, b.get(key), strict, stack),
      )
    )
  if (a instanceof Set && b instanceof Set)
    return (
      a.size === b.size &&
      [...a].every(
        (value) =>
          b.has(value) ||
          [...b].some((other) => equals(value, other, strict, stack)),
      )
    )
  if (Array.isArray(a) && Array.isArray(b))
    return (
      a.length === b.length &&
      a.every(
        (_, index) =>
          (!strict || index in a === index in b) &&
          equals(a[index], b[index], strict, stack),
      )
    )
  // toEqual ignores keys whose value is undefined; toStrictEqual does not.
  const keys = (value: object): string[] =>
    Object.keys(value).filter(
      (key) => strict || Reflect.get(value, key) !== undefined,
    )
  const aKeys = keys(a)
  return (
    aKeys.length === keys(b).length &&
    aKeys.every(
      (key) =>
        Object.hasOwn(b, key) &&
        equals(Reflect.get(a, key), Reflect.get(b, key), strict, stack),
    )
  )
}

/** Jest's `subsetEquality` for toMatchObject. */
export function matchesObject(received: unknown, expected: unknown): boolean {
  if (expected instanceof AsymmetricMatcher)
    return expected.asymmetricMatch(received)
  if (
    typeof expected !== 'object' ||
    expected === null ||
    expected instanceof Date ||
    expected instanceof RegExp
  )
    return equals(received, expected)
  if (typeof received !== 'object' || received === null) return false
  if (Array.isArray(expected))
    return (
      Array.isArray(received) &&
      received.length === expected.length &&
      expected.every((item, index) => matchesObject(received[index], item))
    )
  return Object.entries(expected).every(
    ([key, item]) =>
      key in received && matchesObject(Reflect.get(received, key), item),
  )
}
