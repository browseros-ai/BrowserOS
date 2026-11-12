/**
 * The retrying matchers: for each Locator and Page matcher, the `FrameExpectParams`, argument
 * text and printed expectation matchers.ts builds at v1.64.1, and the op that sends them. Rust
 * polls, prints Received, Timeout and the call log, and owns the failure.
 */
import type { ExpectParams } from '../probe/protocol'
import { parseTemplate, unshift } from './aria-template'
import type { CallSite } from './call-site'
import { plainError, prefixed } from './errors'
import { receiverText, stringify } from './expect-equality'
import { fail, header, type State } from './expect-values'
import { send } from './generated'
import { Locator } from './locator'
import { Page } from './page'
import { type TimeoutOption, timeoutMs } from './timeouts'
import { normalizeUrl } from './url'

export type Retrying = {
  /** What upstream's `expectTypes` accepts; toMatchAriaSnapshot takes either. */
  receiver: 'Locator' | 'Page' | 'Page or Locator'
  /** The matcher's parenthesized argument in the failure header. */
  expectation: string
  /** Playwright's printed expectation, minus `not`. */
  expected: (isNot: boolean) => string
  /**
   * `matcherResult.expected`: the state word of toBeVisible and the other state matchers
   * (`'visible'`, `'checked'`), else the script's own expected value.
   */
  value: unknown
  params: Omit<ExpectParams, 'isNot'>
  options: TimeoutOption | undefined
  /**
   * An aria template that did not parse. Upstream's server throws it as the matcher's failure
   * before it polls; Rust fails the op with it as the `Error:` line, after the call log's title.
   */
  templateError?: string
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
    value: word,
    params: { expression, ...extra },
    options,
  }
}

/**
 * A check of a retrying matcher's own arguments or its receiver, before its op: a refusal under
 * the matcher, `expect.<matcher>`, the name its op runs as.
 */
function argumentError(matcher: string, message: string): Error {
  return plainError(`expect.${matcher}`, message)
}

/** toHaveText and the other string-or-pattern matchers (upstream's toMatchText). */
function text(
  matcher: string,
  receiver: 'Locator' | 'Page',
  expression: string,
  expected: string | RegExp,
  values: ExpectedTextValue[],
  options: (TimeoutOption & { useInnerText?: boolean }) | undefined,
  extra: Partial<ExpectParams> = {},
): Retrying {
  if (typeof expected !== 'string' && !(expected instanceof RegExp))
    throw argumentError(
      matcher,
      'expected value must be a string or regular expression',
    )
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
    value: expected,
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
    value: expected,
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
          'toContainText',
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
      'toHaveAccessibleDescription',
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
      'toHaveAccessibleName',
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
      'toHaveAccessibleErrorMessage',
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
      'toHaveAttribute',
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
          'toHaveClass',
          'Locator',
          'to.have.class',
          expected,
          textValues([expected]),
          options,
        ),
  toContainClass: (expected: string | string[], options?: TimeoutOption) => {
    if (Array.isArray(expected)) {
      if (expected.some((item: unknown) => item instanceof RegExp))
        throw argumentError(
          'toContainClass',
          '"expected" argument in toContainClass cannot contain RegExp values',
        )
      return equal('to.contain.class.array', expected, options, {
        expectedText: textValues(expected),
      })
    }
    if ((expected as unknown) instanceof RegExp)
      throw argumentError(
        'toContainClass',
        '"expected" argument in toContainClass cannot be a RegExp value',
      )
    return text(
      'toContainClass',
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
    text(
      'toHaveCSS',
      'Locator',
      'to.have.css',
      expected,
      textValues([expected]),
      options,
      {
        expressionArg: name,
        ...(options?.pseudo ? { pseudo: options.pseudo } : {}),
      },
    ),
  toHaveId: (expected: string | RegExp, options?: TimeoutOption) =>
    text(
      'toHaveId',
      'Locator',
      'to.have.id',
      expected,
      textValues([expected]),
      options,
    ),
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
      throw argumentError(
        'toHaveRole',
        '"role" argument in toHaveRole must be a string',
      )
    return text(
      'toHaveRole',
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
          'toHaveText',
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
    text(
      'toHaveValue',
      'Locator',
      'to.have.value',
      expected,
      textValues([expected]),
      options,
    ),
  toHaveValues: (expected: (string | RegExp)[], options?: TimeoutOption) =>
    equal('to.have.values', expected, options, {
      expectedText: textValues(expected),
    }),
  toHaveTitle: (expected: string | RegExp, options?: TimeoutOption) =>
    text(
      'toHaveTitle',
      'Page',
      'to.have.title',
      expected,
      textValues([expected], { normalizeWhiteSpace: true }),
      options,
    ),
  toMatchAriaSnapshot: (expected?: unknown, options?: TimeoutOption) => {
    // Upstream reads a template file when given no string, and writes a baseline when the
    // template is empty. Neo has no snapshot files, and a run has no test to update.
    if (typeof expected !== 'string')
      throw argumentError(
        'toMatchAriaSnapshot',
        'snapshot files are not available in Neo; pass the expected snapshot as a YAML string',
      )
    const template = unshift(expected)
    if (!template)
      throw argumentError(
        'toMatchAriaSnapshot',
        'the expected snapshot is empty, and Neo writes no baselines; read the current one with await locator.ariaSnapshot()',
      )
    const parsed = parseTemplate(template)
    return {
      receiver: 'Page or Locator',
      expectation: 'expected',
      expected: (isNot) =>
        `Expected: ${isNot ? 'not ' : ''}${stringify(template)}`,
      value: template,
      params:
        'template' in parsed
          ? { expression: 'to.match.aria', expectedValue: parsed.template }
          : { expression: 'to.match.aria' },
      templateError: 'error' in parsed ? parsed.error : undefined,
      options,
    }
  },
  toHaveURL: (
    expected: string | RegExp,
    options?: TimeoutOption & { ignoreCase?: boolean },
  ) => {
    const value =
      typeof expected === 'string' ? normalizeUrl(expected) : expected
    return text(
      'toHaveURL',
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
  site: CallSite,
  actual: unknown,
  state: State,
  matcher: Retrying,
): Promise<void> {
  const api = `expect.${name}`
  const isPage = actual instanceof Page
  const isLocator = actual instanceof Locator
  const accepted =
    matcher.receiver === 'Page'
      ? isPage
      : matcher.receiver === 'Locator'
        ? isLocator
        : isPage || isLocator
  if (!accepted)
    throw argumentError(
      name,
      `${name} can be only used with ${matcher.receiver} object${matcher.receiver.includes(' or ') ? 's' : ''}, was called with ${receiverText(actual)}`,
    )
  const timeout = matcher.options?.timeout ?? state.config.timeout
  const call = { api, timeoutMs: timeoutMs(api, timeout) }
  // Freeze the payload before resolving a page brand can await: .not and diagnostics belong
  // to this matcher call even when another script turn changes its configuration. `params` are
  // Frame.expect's own, sent flat as upstream's client sends them; the four fields after them
  // are what Rust prints the failure from.
  const expectation = matcher.expectation
  const expected = matcher.expected(state.isNot)
  const params = { ...matcher.params, isNot: state.isNot }
  const templateError = matcher.templateError
  try {
    if (actual instanceof Page)
      await send['expect.page'](site, call, {
        page: await actual._brand(api),
        ...params,
        matcher: name,
        expectation,
        expected,
        templateError,
      })
    else if (actual instanceof Locator)
      await send['expect.locator'](site, call, {
        target: await actual._frame._locatorBrand(api, actual._selector),
        ...params,
        matcher: name,
        expectation,
        expected,
        templateError,
      })
  } catch (error) {
    if (error instanceof Error)
      completeMatcherResult(error, name, state, matcher, timeout)
    if (state.message && error instanceof Error)
      error.message = `${state.message}\n\n${error.message}`
    throw error
  }
}

/** What Rust's expect op saw when the matcher failed (`MatcherFacts`, expectation.rs). */
type MatcherFacts = {
  actual?: unknown
  ariaSnapshot?: string
  log: string[]
  timedOut: boolean
}

function isMatcherFacts(value: unknown): value is MatcherFacts {
  return (
    typeof value === 'object' &&
    value !== null &&
    Array.isArray(Reflect.get(value, 'log')) &&
    typeof Reflect.get(value, 'timedOut') === 'boolean'
  )
}

/**
 * Playwright's `error.matcherResult` (matcherHint.ts, toBeTruthy.ts, toMatchText.ts and
 * toEqual.ts at v1.64.1): Rust's facts, plus what only the script holds. `pass` is what the
 * failing matcher answered, so `true` under `.not`; `message` is the matcher's own, before an
 * `expect(x, message)` prefix; `timeout` is set only when the timeout ended the matcher.
 */
function completeMatcherResult(
  error: Error,
  name: string,
  state: State,
  matcher: Retrying,
  timeout: number,
): void {
  const facts: unknown = Reflect.get(error, 'matcherResult')
  if (!isMatcherFacts(facts)) return
  Reflect.set(error, 'matcherResult', {
    message: error.message,
    pass: state.isNot,
    actual: facts.actual,
    name,
    expected: matcher.value,
    log: facts.log,
    timeout: facts.timedOut ? timeout : undefined,
    ariaSnapshot: facts.ariaSnapshot,
  })
}

/** `toHaveURL(predicate)`: the predicate on each URL the page commits, until it passes. */
export async function urlPredicate(
  page: Page,
  site: CallSite,
  predicate: (url: unknown) => unknown,
  state: State,
  options: TimeoutOption | undefined,
): Promise<void> {
  const timeout = options?.timeout ?? state.config.timeout
  const deadline = Date.now() + timeout
  for (;;) {
    let passed: boolean
    try {
      passed = Boolean(await predicate(URL.parse(page.url()) ?? page.url()))
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
        site,
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
