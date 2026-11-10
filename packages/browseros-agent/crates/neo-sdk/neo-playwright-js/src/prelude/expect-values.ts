/**
 * The value matchers (`toBe`, `toEqual`, `toThrow`, ...), which never touch the page: Jest's
 * semantics and message shape, run in the prelude. `State` is one assertion's modifiers.
 */
import { assertionError } from './errors'
import {
  AsymmetricMatcher,
  closeTo,
  equals,
  matchesObject,
  stringify,
} from './expect-equality'

export type ExpectConfig = { timeout: number }
export type Modifier = '' | 'resolves' | 'rejects'
export type State = {
  isNot: boolean
  promise: Modifier
  config: ExpectConfig
  message: string | undefined
}
export type Matcher = (...args: never[]) => unknown

export function header(
  name: string,
  state: State,
  receiver = 'received',
  args = 'expected',
): string {
  const modifier = state.promise ? `.${state.promise}` : ''
  return `expect(${receiver})${modifier}${state.isNot ? '.not' : ''}.${name}(${args})`
}

export function fail(state: State, message: string): never {
  throw assertionError(
    state.message ? `${state.message}\n\n${message}` : message,
  )
}

/** Checks a value matcher's result against `.not`, failing with Jest's message. */
function check(state: State, pass: boolean, message: () => string): void {
  if (pass !== state.isNot) return
  fail(state, message())
}

function expectedLine(state: State, value: unknown): string {
  return `Expected: ${state.isNot ? 'not ' : ''}${stringify(value)}`
}

export function valueMatchers(
  actual: unknown,
  state: State,
): Record<string, Matcher> {
  const plain = (name: string, args = 'expected') =>
    header(name, state, 'received', args)
  return {
    toBe: (expected: unknown) =>
      check(state, Object.is(actual, expected), () =>
        [
          `${plain('toBe')} // Object.is equality`,
          '',
          expectedLine(state, expected),
          ...(state.isNot ? [] : [`Received: ${stringify(actual)}`]),
        ].join('\n'),
      ),
    toEqual: (expected: unknown) =>
      check(state, equals(actual, expected), () =>
        [
          `${plain('toEqual')} // deep equality`,
          '',
          expectedLine(state, expected),
          ...(state.isNot ? [] : [`Received: ${stringify(actual)}`]),
        ].join('\n'),
      ),
    toStrictEqual: (expected: unknown) =>
      check(state, equals(actual, expected, true), () =>
        [
          `${plain('toStrictEqual')} // deep equality`,
          '',
          expectedLine(state, expected),
          ...(state.isNot ? [] : [`Received: ${stringify(actual)}`]),
        ].join('\n'),
      ),
    toBeTruthy: () =>
      check(
        state,
        Boolean(actual),
        () => `${plain('toBeTruthy', '')}\n\nReceived: ${stringify(actual)}`,
      ),
    toBeFalsy: () =>
      check(
        state,
        !actual,
        () => `${plain('toBeFalsy', '')}\n\nReceived: ${stringify(actual)}`,
      ),
    toBeNull: () =>
      check(
        state,
        actual === null,
        () => `${plain('toBeNull', '')}\n\nReceived: ${stringify(actual)}`,
      ),
    toBeUndefined: () =>
      check(
        state,
        actual === undefined,
        () => `${plain('toBeUndefined', '')}\n\nReceived: ${stringify(actual)}`,
      ),
    toBeDefined: () =>
      check(
        state,
        actual !== undefined,
        () => `${plain('toBeDefined', '')}\n\nReceived: ${stringify(actual)}`,
      ),
    toBeNaN: () =>
      check(
        state,
        Number.isNaN(actual),
        () => `${plain('toBeNaN', '')}\n\nReceived: ${stringify(actual)}`,
      ),
    toBeGreaterThan: (expected: number) =>
      compare(state, actual, expected, '>', 'toBeGreaterThan'),
    toBeGreaterThanOrEqual: (expected: number) =>
      compare(state, actual, expected, '>=', 'toBeGreaterThanOrEqual'),
    toBeLessThan: (expected: number) =>
      compare(state, actual, expected, '<', 'toBeLessThan'),
    toBeLessThanOrEqual: (expected: number) =>
      compare(state, actual, expected, '<=', 'toBeLessThanOrEqual'),
    toBeCloseTo: (expected: number, precision = 2) =>
      check(
        state,
        typeof actual === 'number' && closeTo(actual, expected, precision),
        () =>
          `${plain('toBeCloseTo', 'expected, precision')}\n\nExpected: ${state.isNot ? 'not ' : ''}${stringify(expected)}\nReceived: ${stringify(actual)}\n\nExpected precision:    ${precision}`,
      ),
    toBeInstanceOf: (expected: unknown) =>
      check(
        state,
        typeof expected === 'function' && actual instanceof expected,
        () =>
          `${plain('toBeInstanceOf')}\n\nExpected constructor: ${state.isNot ? 'not ' : ''}${typeof expected === 'function' ? expected.name : stringify(expected)}\nReceived value: ${stringify(actual)}`,
      ),
    toContain: (expected: unknown) => {
      const pass =
        typeof actual === 'string'
          ? actual.includes(String(expected))
          : actual !== null &&
            actual !== undefined &&
            typeof Reflect.get(Object(actual), Symbol.iterator) ===
              'function' &&
            [...(actual as Iterable<unknown>)].includes(expected)
      check(
        state,
        pass,
        () =>
          `${plain('toContain')} // indexOf\n\nExpected value: ${state.isNot ? 'not ' : ''}${stringify(expected)}\nReceived: ${stringify(actual)}`,
      )
    },
    toContainEqual: (expected: unknown) => {
      const pass =
        actual !== null &&
        actual !== undefined &&
        typeof Reflect.get(Object(actual), Symbol.iterator) === 'function' &&
        [...(actual as Iterable<unknown>)].some((item) =>
          equals(item, expected),
        )
      check(
        state,
        pass,
        () =>
          `${plain('toContainEqual')} // deep equality\n\nExpected value: ${state.isNot ? 'not ' : ''}${stringify(expected)}\nReceived: ${stringify(actual)}`,
      )
    },
    toHaveLength: (expected: number) => {
      const length: unknown =
        actual === null || actual === undefined
          ? undefined
          : Reflect.get(Object(actual), 'length')
      check(
        state,
        length === expected,
        () =>
          `${plain('toHaveLength')}\n\nExpected length: ${state.isNot ? 'not ' : ''}${expected}\nReceived length: ${String(length)}\nReceived: ${stringify(actual)}`,
      )
    },
    toHaveProperty: (
      path: string | (string | number)[],
      ...value: unknown[]
    ) => {
      const keys = Array.isArray(path) ? path : path.split('.')
      let current: unknown = actual
      let found = true
      for (const key of keys) {
        if (
          current === null ||
          current === undefined ||
          !(String(key) in Object(current))
        ) {
          found = false
          break
        }
        current = Reflect.get(Object(current), key)
      }
      const pass = found && (value.length === 0 || equals(current, value[0]))
      check(
        state,
        pass,
        () =>
          `${plain('toHaveProperty', value.length ? 'path, value' : 'path')}\n\nExpected path: ${stringify(path)}${value.length ? `\nExpected value: ${state.isNot ? 'not ' : ''}${stringify(value[0])}` : ''}\nReceived: ${found ? stringify(current) : stringify(actual)}`,
      )
    },
    toMatch: (expected: string | RegExp) => {
      if (typeof actual !== 'string')
        fail(
          state,
          `${plain('toMatch')}\n\nreceived value must be a string\nReceived: ${stringify(actual)}`,
        )
      const pass =
        typeof expected === 'string'
          ? actual.includes(expected)
          : new RegExp(expected).test(actual)
      check(
        state,
        pass,
        () =>
          `${plain('toMatch')}\n\nExpected ${typeof expected === 'string' ? 'substring' : 'pattern'}: ${state.isNot ? 'not ' : ''}${stringify(expected)}\nReceived string: ${stringify(actual)}`,
      )
    },
    toMatchObject: (expected: unknown) =>
      check(
        state,
        matchesObject(actual, expected),
        () =>
          `${plain('toMatchObject')}\n\n${expectedLine(state, expected)}\nReceived: ${stringify(actual)}`,
      ),
    toThrow: (expected?: unknown) => thrown(actual, expected, state, 'toThrow'),
    toThrowError: (expected?: unknown) =>
      thrown(actual, expected, state, 'toThrowError'),
  }
}

function compare(
  state: State,
  actual: unknown,
  expected: number,
  operator: '>' | '>=' | '<' | '<=',
  name: string,
): void {
  const pass =
    typeof actual === 'number' || typeof actual === 'bigint'
      ? operator === '>'
        ? actual > expected
        : operator === '>='
          ? actual >= expected
          : operator === '<'
            ? actual < expected
            : actual <= expected
      : false
  check(
    state,
    pass,
    () =>
      `${header(name, state)}\n\nExpected: ${state.isNot ? 'not ' : ''}${operator} ${stringify(expected)}\nReceived:${' '.repeat(operator.length + 1)}${stringify(actual)}`,
  )
}

/** toThrow: with `rejects`, the rejection reason; otherwise what calling `actual` throws. */
function thrown(
  actual: unknown,
  expected: unknown,
  state: State,
  name: string,
): void {
  let error: unknown
  let didThrow = false
  if (state.promise === 'rejects') {
    error = actual
    didThrow = true
  } else {
    if (typeof actual !== 'function')
      fail(
        state,
        `${header(name, state)}\n\nReceived value must be a function\nReceived: ${stringify(actual)}`,
      )
    try {
      Reflect.apply(actual, undefined, [])
    } catch (caught) {
      error = caught
      didThrow = true
    }
  }
  const message =
    error instanceof Error
      ? error.message
      : typeof error === 'string'
        ? error
        : stringify(error)
  let pass = didThrow
  if (didThrow && expected !== undefined) {
    if (typeof expected === 'string') pass = message.includes(expected)
    else if (expected instanceof RegExp) pass = expected.test(message)
    else if (typeof expected === 'function') pass = error instanceof expected
    else if (expected instanceof AsymmetricMatcher)
      pass = expected.asymmetricMatch(error)
    else if (expected instanceof Error) pass = message === expected.message
  }
  check(
    state,
    pass,
    () =>
      `${header(name, state)}\n\n${expected === undefined ? '' : `Expected: ${state.isNot ? 'not ' : ''}${stringify(expected)}\n`}${didThrow ? `Received message: ${stringify(message)}` : 'Received function did not throw'}`,
  )
}
