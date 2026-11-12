/**
 * Playwright Test's `expect`, for scripts.
 *
 * Retrying matchers on a Locator or a Page are one op each (`expect.locator`, `expect.page`):
 * the prelude sends upstream's `FrameExpectParams` (matchers.ts at v1.64.1), the matcher's
 * name and argument text, and the `Expected:` line Playwright would print. Rust polls with
 * upstream's backoff, adds `Locator:`, `Received:`, `Timeout:` and the call log, and owns the
 * failure, so the trace record and the script's error say the same thing.
 *
 * Value matchers (`toBe`, `toEqual`, ...) never touch the page and run in the prelude with
 * Jest's semantics (expect-values.ts). `expect.poll` and `toPass` retry here.
 */
import { sleep } from './bridge'
import { type CallSite, captureSite } from './call-site'
import { assertionError, plainError, unsupportedError } from './errors'
import {
  Any,
  Anything,
  ArrayContaining,
  ArrayOf,
  CloseTo,
  ObjectContaining,
  receiverText,
  StringContaining,
  StringMatching,
  stringify,
} from './expect-equality'
import {
  hrefOf,
  RETRYING,
  type Retrying,
  retry,
  urlPredicate,
} from './expect-retrying'
import {
  type ExpectConfig,
  fail,
  header,
  type Matcher,
  type State,
  valueMatchers,
} from './expect-values'
import { Page } from './page'
import { DEFAULT_TIMEOUT, type TimeoutOption } from './timeouts'
import { installUnsupportedExpect } from './unsupported'

/** One assertion's matchers, with `not`, `resolves` and `rejects` as getters. */
export type Matchers = Record<string, unknown>

/** What toHaveURL reads of a `URLPattern`: its own `test`. */
type URLPatternLike = { test(input: string): boolean }

/**
 * A `URLPattern`, read by its brand since QuickJS has no such global and a script's pattern may
 * come from another realm (Node's, in the spec harness); upstream checks `instanceof`.
 */
function isURLPattern(value: unknown): value is URLPatternLike {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof Reflect.get(value, 'test') === 'function' &&
    Object.prototype.toString.call(value) === '[object URLPattern]'
  )
}

/**
 * A retrying matcher's call: the Page predicate forms of toHaveURL (a function, or a URLPattern
 * that upstream turns into one), or one op.
 */
function retrying(
  name: string,
  site: CallSite,
  actual: unknown,
  state: State,
  args: never[],
): Promise<void> {
  const expected: unknown = args[0]
  if (
    name === 'toHaveURL' &&
    (typeof expected === 'function' || isURLPattern(expected))
  ) {
    if (!(actual instanceof Page))
      throw plainError(
        'expect.toHaveURL',
        `toHaveURL can be only used with Page object, was called with ${receiverText(actual)}`,
      )
    const predicate = isURLPattern(expected)
      ? (url: unknown) => expected.test(hrefOf(url))
      : (expected as (url: unknown) => unknown)
    return urlPredicate(
      actual,
      site,
      predicate,
      state,
      args[1] as TimeoutOption | undefined,
    )
  }
  const build = RETRYING[name] as (...args: never[]) => Retrying
  return retry(name, site, actual, state, Reflect.apply(build, undefined, args))
}

function matchers(actual: unknown, state: State): Matchers {
  const built: Record<string, unknown> = {}
  for (const name of Object.keys(valueMatchers(actual, state)))
    built[name] = (...args: never[]) => {
      const run = (value: unknown) =>
        Reflect.apply(
          valueMatchers(value, state)[name] as Matcher,
          undefined,
          args,
        )
      return state.promise ? settle(actual, state, run, name) : run(actual)
    }
  for (const name of Object.keys(RETRYING))
    built[name] = (...args: never[]) => {
      const site = captureSite()
      return state.promise
        ? settle(
            actual,
            state,
            (value) => retrying(name, site, value, state, args),
            name,
          )
        : retrying(name, site, actual, state, args)
    }
  built.toPass = (options?: { timeout?: number; intervals?: number[] }) =>
    toPass(actual, state, options)
  installUnsupportedExpect(built, 'matchers')
  Object.defineProperty(built, 'not', {
    get: () => matchers(actual, { ...state, isNot: !state.isNot }),
  })
  Object.defineProperty(built, 'resolves', {
    get: () => matchers(actual, { ...state, promise: 'resolves' }),
  })
  Object.defineProperty(built, 'rejects', {
    get: () => matchers(actual, { ...state, promise: 'rejects' }),
  })
  return built as Matchers
}

/** `resolves` / `rejects`: settle the promise first, then match its value or reason. */
async function settle(
  actual: unknown,
  state: State,
  match: (value: unknown) => unknown,
  name: string,
): Promise<void> {
  const subject =
    typeof actual === 'function' ? Reflect.apply(actual, undefined, []) : actual
  if (
    !(subject instanceof Promise) &&
    typeof Reflect.get(Object(subject), 'then') !== 'function'
  )
    fail(
      state,
      `${header(name, state)}\n\nreceived value must be a promise or a function returning a promise\nReceived: ${stringify(actual)}`,
    )
  let value: unknown
  try {
    value = await subject
  } catch (reason) {
    if (state.promise === 'resolves')
      fail(
        state,
        `${header(name, state)}\n\nReceived promise rejected instead of resolved\nRejected to value: ${stringify(reason)}`,
      )
    await match(reason)
    return
  }
  if (state.promise === 'rejects')
    fail(
      state,
      `${header(name, state)}\n\nReceived promise resolved instead of rejected\nResolved to value: ${stringify(value)}`,
    )
  await match(value)
}

const INTERVALS = [100, 250, 500, 1000]

/** `expect(callback).toPass()`: upstream's default timeout is 0, retrying until the run ends. */
async function toPass(
  callback: unknown,
  state: State,
  options: { timeout?: number; intervals?: number[] } = {},
): Promise<void> {
  if (typeof callback !== 'function')
    throw plainError('expect.toPass', 'toPass: expected a function')
  const timeout = options.timeout ?? 0
  const intervals = options.intervals ?? INTERVALS
  const deadline = timeout ? Date.now() + timeout : Number.POSITIVE_INFINITY
  for (let attempt = 0; ; attempt++) {
    let error: unknown
    try {
      await Reflect.apply(callback, undefined, [])
    } catch (caught) {
      error = caught
    }
    const passed = error === undefined
    if (passed !== state.isNot) return
    const wait = intervals[Math.min(attempt, intervals.length - 1)] ?? 1000
    if (Date.now() + wait > deadline) {
      const last = error instanceof Error ? `${error.message}\n\n` : ''
      fail(
        state,
        `${last}Call Log:\n- Timeout ${timeout}ms exceeded while waiting on the predicate`,
      )
    }
    await sleep('expect.toPass', wait)
  }
}

/** `expect.poll(fn)`: value matchers on what `fn` returns, retried until they pass. */
function poll(
  generator: unknown,
  state: State,
  options: { timeout?: number; intervals?: number[] } = {},
): Matchers {
  if (typeof generator !== 'function')
    throw plainError('expect.poll', 'expect.poll: expected a function')
  const timeout = options.timeout ?? state.config.timeout
  const intervals = options.intervals ?? INTERVALS
  const polled = (isNot: boolean): Matchers => {
    const result: Record<string, Matcher> = {}
    for (const name of Object.keys(valueMatchers(undefined, state)))
      result[name] = (...args: never[]) =>
        (async () => {
          const deadline = Date.now() + timeout
          for (let attempt = 0; ; attempt++) {
            const value = await Reflect.apply(generator, undefined, [])
            try {
              Reflect.apply(
                valueMatchers(value, { ...state, isNot })[name] as Matcher,
                undefined,
                args,
              )
              return
            } catch (error) {
              const wait =
                intervals[Math.min(attempt, intervals.length - 1)] ?? 1000
              if (timeout !== 0 && Date.now() + wait > deadline) {
                const message =
                  error instanceof Error ? error.message : String(error)
                throw assertionError(
                  `${message}\n\nCall Log:\n- Timeout ${timeout}ms exceeded while waiting on the predicate`,
                )
              }
              await sleep('expect.poll', wait)
            }
          }
        })()
    const built = result as Matchers
    Object.defineProperty(built, 'not', { get: () => polled(!isNot) })
    return built
  }
  return polled(state.isNot)
}

export type Expect = ((
  actual: unknown,
  messageOrOptions?: string | { message?: string },
) => Matchers) &
  Record<string, unknown>

/** Builds `expect` with its statics; `configure` returns another with new defaults. */
export function createExpect(
  config: ExpectConfig = { timeout: DEFAULT_TIMEOUT },
): Expect {
  const expect = ((
    actual: unknown,
    messageOrOptions?: string | { message?: string },
  ) => {
    const message =
      typeof messageOrOptions === 'string'
        ? messageOrOptions
        : messageOrOptions?.message
    return matchers(actual, { isNot: false, promise: '', config, message })
  }) as Expect
  expect.any = (expected: unknown) => new Any(expected)
  expect.anything = () => new Anything(false)
  expect.arrayContaining = (expected: unknown[]) =>
    new ArrayContaining(expected, false)
  expect.arrayOf = (expected: unknown) => new ArrayOf(expected, false)
  expect.objectContaining = (expected: Record<string, unknown>) =>
    new ObjectContaining(expected, false)
  expect.stringContaining = (expected: string) =>
    new StringContaining(expected, false)
  expect.stringMatching = (expected: string | RegExp) =>
    new StringMatching(expected, false)
  expect.closeTo = (expected: number, precision = 2) =>
    new CloseTo(expected, precision, false)
  expect.not = {
    arrayContaining: (expected: unknown[]) =>
      new ArrayContaining(expected, true),
    arrayOf: (expected: unknown) => new ArrayOf(expected, true),
    objectContaining: (expected: Record<string, unknown>) =>
      new ObjectContaining(expected, true),
    stringContaining: (expected: string) =>
      new StringContaining(expected, true),
    stringMatching: (expected: string | RegExp) =>
      new StringMatching(expected, true),
    closeTo: (expected: number, precision = 2) =>
      new CloseTo(expected, precision, true),
  }
  expect.configure = (
    options: { timeout?: number; soft?: boolean; message?: string } = {},
  ) => {
    if (options.soft)
      throw unsupportedError(
        'expect.configure',
        'soft assertions are not available in Neo; catch the error to continue',
      )
    return createExpect({ timeout: options.timeout ?? config.timeout })
  }
  expect.poll = (
    generator: unknown,
    options?: { timeout?: number; intervals?: number[]; message?: string },
  ) =>
    poll(
      generator,
      { isNot: false, promise: '', config, message: options?.message },
      options,
    )
  installUnsupportedExpect(expect, 'statics')
  return expect
}
