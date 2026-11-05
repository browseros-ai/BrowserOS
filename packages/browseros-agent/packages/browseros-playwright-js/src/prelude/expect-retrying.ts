/**
 * The retrying matchers: for each Locator and Page matcher, the `FrameExpectParams`, argument
 * text and printed expectation matchers.ts builds at v1.64.1, and the op that sends them. Rust
 * polls, prints Received, Timeout and the call log, and owns the failure.
 */
import type { ExpectParams } from '../probe/protocol'
import { invoke } from './bridge'
import { prefixed } from './errors'
import { receiverText, stringify } from './expect-equality'
import { fail, header, type State } from './expect-values'
import { Locator } from './locator'
import { Page } from './page'
import { type TimeoutOption, timeoutMs } from './timeouts'
import { normalizeUrl, urlLike } from './url'

export type Retrying = {
  receiver: 'Locator' | 'Page'
  /** The matcher's parenthesized argument in the failure header. */
  expectation: string
  /** Playwright's printed expectation, minus `not`. */
  expected: (isNot: boolean) => string
  params: Omit<ExpectParams, 'isNot'>
  options: TimeoutOption | undefined
}

type ExpectedTextValue = NonNullable<ExpectParams['expectedText']>[number]

function textValues(
  items: (string | RegExp)[],
  options: {
    matchSubstring?: boolean
    normalizeWhiteSpace?: boolean
    ignoreCase?: boolean
  } = {},
): ExpectedTextValue[] {
  return items.map((item) => {
    const value: ExpectedTextValue = {}
    if (typeof item === 'string') value.string = item
    else {
      value.regexSource = item.source
      value.regexFlags = item.flags
    }
    if (options.matchSubstring !== undefined)
      value.matchSubstring = options.matchSubstring
    if (options.ignoreCase !== undefined) value.ignoreCase = options.ignoreCase
    if (options.normalizeWhiteSpace !== undefined)
      value.normalizeWhiteSpace = options.normalizeWhiteSpace
    return value
  })
}

/** toBeVisible and the other state matchers (upstream's toBeTruthy). */
function truthy(
  expression: string,
  word: string,
  arg: string,
  options: TimeoutOption | undefined,
  extra: Partial<ExpectParams> = {},
): Retrying {
  return {
    receiver: 'Locator',
    expectation: arg,
    expected: (isNot) => `Expected: ${isNot ? 'not ' : ''}${word}`,
    params: { expression, ...extra },
    options,
  }
}

/** toHaveText and the other string-or-pattern matchers (upstream's toMatchText). */
function text(
  receiver: 'Locator' | 'Page',
  expression: string,
  expected: string | RegExp,
  values: ExpectedTextValue[],
  options: (TimeoutOption & { useInnerText?: boolean }) | undefined,
  extra: Partial<ExpectParams> = {},
): Retrying {
  if (typeof expected !== 'string' && !(expected instanceof RegExp))
    throw new Error('expected value must be a string or regular expression')
  const substring = values[0]?.matchSubstring === true
  const suffix =
    typeof expected === 'string' ? (substring ? ' substring' : '') : ' pattern'
  const params: Omit<ExpectParams, 'isNot'> = {
    expression,
    expectedText: values,
    ...extra,
  }
  if (options?.useInnerText !== undefined)
    params.useInnerText = options.useInnerText
  return {
    receiver,
    expectation: 'expected',
    expected: (isNot) =>
      `Expected${suffix}: ${isNot ? 'not ' : ''}${stringify(expected)}`,
    params,
    options,
  }
}

/** toHaveCount, the array forms and toHaveJSProperty (upstream's toEqual). */
function equal(
  expression: string,
  expected: unknown,
  options: (TimeoutOption & { useInnerText?: boolean }) | undefined,
  extra: Partial<ExpectParams> = {},
): Retrying {
  const params: Omit<ExpectParams, 'isNot'> = { expression, ...extra }
  if (options?.useInnerText !== undefined)
    params.useInnerText = options.useInnerText
  return {
    receiver: 'Locator',
    expectation: 'expected',
    expected: (isNot) =>
      `Expected: ${isNot ? 'not ' : ''}${stringify(expected)}`,
    params,
    options,
  }
}

type TextOptions = TimeoutOption & {
  ignoreCase?: boolean
  useInnerText?: boolean
}

/** Each retrying matcher's params, as matchers.ts builds them at v1.64.1. */
export const RETRYING: Record<string, (...args: never[]) => Retrying> = {
  toBeAttached: (options?: TimeoutOption & { attached?: boolean }) => {
    const attached = options?.attached !== false
    return truthy(
      attached ? 'to.be.attached' : 'to.be.detached',
      attached ? 'attached' : 'detached',
      attached ? '' : '{ attached: false }',
      options,
    )
  },
  toBeChecked: (
    options?: TimeoutOption & { checked?: boolean; indeterminate?: boolean },
  ) => {
    const expectedValue: Record<string, boolean> = {}
    if (options?.checked !== undefined) expectedValue.checked = options.checked
    if (options?.indeterminate !== undefined)
      expectedValue.indeterminate = options.indeterminate
    const word = options?.indeterminate
      ? 'indeterminate'
      : options?.checked === false
        ? 'unchecked'
        : 'checked'
    const arg = options?.indeterminate
      ? '{ indeterminate: true }'
      : options?.checked === false
        ? '{ checked: false }'
        : ''
    return truthy('to.be.checked', word, arg, options, { expectedValue })
  },
  toBeDisabled: (options?: TimeoutOption) =>
    truthy('to.be.disabled', 'disabled', '', options),
  toBeEditable: (options?: TimeoutOption & { editable?: boolean }) => {
    const editable = options?.editable !== false
    return truthy(
      editable ? 'to.be.editable' : 'to.be.readonly',
      editable ? 'editable' : 'readOnly',
      editable ? '' : '{ editable: false }',
      options,
    )
  },
  toBeEmpty: (options?: TimeoutOption) =>
    truthy('to.be.empty', 'empty', '', options),
  toBeEnabled: (options?: TimeoutOption & { enabled?: boolean }) => {
    const enabled = options?.enabled !== false
    return truthy(
      enabled ? 'to.be.enabled' : 'to.be.disabled',
      enabled ? 'enabled' : 'disabled',
      enabled ? '' : '{ enabled: false }',
      options,
    )
  },
  toBeFocused: (options?: TimeoutOption) =>
    truthy('to.be.focused', 'focused', '', options),
  toBeHidden: (options?: TimeoutOption) =>
    truthy('to.be.hidden', 'hidden', '', options),
  toBeVisible: (options?: TimeoutOption & { visible?: boolean }) => {
    const visible = options?.visible !== false
    return truthy(
      visible ? 'to.be.visible' : 'to.be.hidden',
      visible ? 'visible' : 'hidden',
      visible ? '' : '{ visible: false }',
      options,
    )
  },
  toBeInViewport: (options?: TimeoutOption & { ratio?: number }) =>
    truthy(
      'to.be.in.viewport',
      'in viewport',
      '',
      options,
      options?.ratio === undefined ? {} : { expectedNumber: options.ratio },
    ),
  toContainText: (
    expected: string | RegExp | (string | RegExp)[],
    options?: TextOptions,
  ) =>
    Array.isArray(expected)
      ? equal('to.contain.text.array', expected, options, {
          expectedText: textValues(expected, {
            matchSubstring: true,
            normalizeWhiteSpace: true,
            ignoreCase: options?.ignoreCase,
          }),
        })
      : text(
          'Locator',
          'to.have.text',
          expected,
          textValues([expected], {
            matchSubstring: true,
            normalizeWhiteSpace: true,
            ignoreCase: options?.ignoreCase,
          }),
          options,
        ),
  toHaveAccessibleDescription: (
    expected: string | RegExp,
    options?: TextOptions,
  ) =>
    text(
      'Locator',
      'to.have.accessible.description',
      expected,
      textValues([expected], {
        ignoreCase: options?.ignoreCase,
        normalizeWhiteSpace: true,
      }),
      options,
    ),
  toHaveAccessibleName: (expected: string | RegExp, options?: TextOptions) =>
    text(
      'Locator',
      'to.have.accessible.name',
      expected,
      textValues([expected], {
        ignoreCase: options?.ignoreCase,
        normalizeWhiteSpace: true,
      }),
      options,
    ),
  toHaveAccessibleErrorMessage: (
    expected: string | RegExp,
    options?: TextOptions,
  ) =>
    text(
      'Locator',
      'to.have.accessible.error.message',
      expected,
      textValues([expected], {
        ignoreCase: options?.ignoreCase,
        normalizeWhiteSpace: true,
      }),
      options,
    ),
  toHaveAttribute: (
    name: string,
    expected?: string | RegExp | TimeoutOption,
    options?: TextOptions,
  ) => {
    if (
      !options &&
      typeof expected === 'object' &&
      expected !== null &&
      !(expected instanceof RegExp)
    )
      return truthy('to.have.attribute', 'have attribute', '', expected, {
        expressionArg: name,
      })
    if (expected === undefined)
      return truthy('to.have.attribute', 'have attribute', '', options, {
        expressionArg: name,
      })
    const value = expected as string | RegExp
    return text(
      'Locator',
      'to.have.attribute.value',
      value,
      textValues([value], { ignoreCase: options?.ignoreCase }),
      options,
      { expressionArg: name },
    )
  },
  toHaveClass: (
    expected: string | RegExp | (string | RegExp)[],
    options?: TimeoutOption,
  ) =>
    Array.isArray(expected)
      ? equal('to.have.class.array', expected, options, {
          expectedText: textValues(expected),
        })
      : text(
          'Locator',
          'to.have.class',
          expected,
          textValues([expected]),
          options,
        ),
  toContainClass: (expected: string | string[], options?: TimeoutOption) => {
    if (Array.isArray(expected)) {
      if (expected.some((item: unknown) => item instanceof RegExp))
        throw new Error(
          '"expected" argument in toContainClass cannot contain RegExp values',
        )
      return equal('to.contain.class.array', expected, options, {
        expectedText: textValues(expected),
      })
    }
    if ((expected as unknown) instanceof RegExp)
      throw new Error(
        '"expected" argument in toContainClass cannot be a RegExp value',
      )
    return text(
      'Locator',
      'to.contain.class',
      expected,
      textValues([expected]),
      options,
    )
  },
  toHaveCount: (expected: number, options?: TimeoutOption) =>
    equal('to.have.count', expected, options, { expectedNumber: expected }),
  toHaveCSS: (
    name: string,
    expected: string | RegExp,
    options?: TimeoutOption & { pseudo?: 'before' | 'after' },
  ) =>
    text('Locator', 'to.have.css', expected, textValues([expected]), options, {
      expressionArg: name,
      ...(options?.pseudo ? { pseudo: options.pseudo } : {}),
    }),
  toHaveId: (expected: string | RegExp, options?: TimeoutOption) =>
    text('Locator', 'to.have.id', expected, textValues([expected]), options),
  toHaveJSProperty: (
    name: string,
    expected: unknown,
    options?: TimeoutOption,
  ) =>
    equal('to.have.property', expected, options, {
      expressionArg: name,
      expectedValue: expected,
    }),
  toHaveRole: (expected: string, options?: TimeoutOption) => {
    if (typeof expected !== 'string')
      throw new Error('"role" argument in toHaveRole must be a string')
    return text(
      'Locator',
      'to.have.role',
      expected,
      textValues([expected]),
      options,
    )
  },
  toHaveText: (
    expected: string | RegExp | (string | RegExp)[],
    options?: TextOptions,
  ) =>
    Array.isArray(expected)
      ? equal('to.have.text.array', expected, options, {
          expectedText: textValues(expected, {
            normalizeWhiteSpace: true,
            ignoreCase: options?.ignoreCase,
          }),
        })
      : text(
          'Locator',
          'to.have.text',
          expected,
          textValues([expected], {
            normalizeWhiteSpace: true,
            ignoreCase: options?.ignoreCase,
          }),
          options,
        ),
  toHaveValue: (expected: string | RegExp, options?: TimeoutOption) =>
    text('Locator', 'to.have.value', expected, textValues([expected]), options),
  toHaveValues: (expected: (string | RegExp)[], options?: TimeoutOption) =>
    equal('to.have.values', expected, options, {
      expectedText: textValues(expected),
    }),
  toHaveTitle: (expected: string | RegExp, options?: TimeoutOption) =>
    text(
      'Page',
      'to.have.title',
      expected,
      textValues([expected], { normalizeWhiteSpace: true }),
      options,
    ),
  toHaveURL: (
    expected: string | RegExp,
    options?: TimeoutOption & { ignoreCase?: boolean },
  ) => {
    const value =
      typeof expected === 'string' ? normalizeUrl(expected) : expected
    return text(
      'Page',
      'to.have.url',
      value,
      textValues([value], { ignoreCase: options?.ignoreCase }),
      options,
    )
  },
}

export async function retry(
  name: string,
  actual: unknown,
  state: State,
  matcher: Retrying,
): Promise<void> {
  const api = `expect.${name}`
  const isPage = actual instanceof Page
  const isLocator = actual instanceof Locator
  if (matcher.receiver === 'Page' ? !isPage : !isLocator)
    throw new Error(
      `${name} can be only used with ${matcher.receiver} object, was called with ${receiverText(actual)}`,
    )
  const call = {
    api,
    timeoutMs: timeoutMs(api, matcher.options?.timeout ?? state.config.timeout),
  }
  const shape = {
    matcher: name,
    expectation: matcher.expectation,
    expected: matcher.expected(state.isNot),
    params: { ...matcher.params, isNot: state.isNot },
  }
  try {
    if (actual instanceof Page)
      await invoke('expect.page', call, {
        page: await actual._brand(api),
        ...shape,
      })
    else if (actual instanceof Locator)
      await invoke('expect.locator', call, {
        target: await actual._frame._locatorBrand(api, actual._selector),
        ...shape,
      })
  } catch (error) {
    if (state.message && error instanceof Error)
      error.message = `${state.message}\n\n${error.message}`
    throw error
  }
}

/** `toHaveURL(predicate)`: the predicate on each URL the page commits, until it passes. */
export async function urlPredicate(
  page: Page,
  predicate: (url: unknown) => unknown,
  state: State,
  options: TimeoutOption | undefined,
): Promise<void> {
  const timeout = options?.timeout ?? state.config.timeout
  const deadline = Date.now() + timeout
  for (;;) {
    let passed: boolean
    try {
      passed = Boolean(await predicate(urlLike(page.url()) ?? page.url()))
    } catch (error) {
      throw prefixed('expect.toHaveURL', error)
    }
    if (passed !== state.isNot) return
    const remaining = deadline - Date.now()
    if (timeout !== 0 && remaining <= 0)
      fail(
        state,
        `${header('toHaveURL', state, 'page')} failed\n\nExpected: predicate to ${state.isNot ? 'fail' : 'succeed'}\nReceived: ${stringify(page.url())}\nTimeout: ${timeout}ms`,
      )
    try {
      await page._waitForURL(
        'expect.toHaveURL',
        page._mainFrame,
        (url) => {
          const href =
            typeof url === 'object' && url !== null
              ? Reflect.get(url, 'href')
              : url
          return href !== page.url()
        },
        { timeout: timeout === 0 ? 0 : remaining, waitUntil: 'commit' },
      )
    } catch (error) {
      if (
        Reflect.get(Object(error), 'timeout') !== true &&
        !(error instanceof Error && error.name === 'TimeoutError')
      )
        throw error
    }
  }
}
