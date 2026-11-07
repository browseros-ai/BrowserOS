/**
 * Playwright's `Frame` (`client/frame.ts` at 26137face): its Rust-minted guid,
 * its page, the selector builders, whose bodies are Playwright's, `goto`, `title`,
 * `textContent`, `innerText`, `getAttribute`, browser actions, and the waits `waitForURL` and
 * `waitForLoadState` (with `_waitForSelector` behind `locator.waitFor`). A page keeps one
 * `Frame` per guid, because `Locator` compares frames by identity.
 *
 * Each `_<method>` is where every public form of a browser call (`locator.textContent`,
 * `page.textContent`, `frame.textContent`) becomes a protocol message, with its caller's
 * `apiName`. Each runs synchronously into `Connection.send`, so the location it captures is the
 * script's call site.
 *
 * `goto` resolves the navigation's `Response` (`network.ts`), or `null` where Playwright's does:
 * a same-document change, or a document without a network request (`about:blank`, `data:`).
 *
 * What differs from Playwright:
 * - Detached frames retain the last URL and name read by a getter or frame record, as Page
 *   retains its last URL after closure; without frame events, unseen changes are not retained.
 * - `waitForURL` and `waitForLoadState` are one `neo:` call each, which Rust waits out
 *   (`neo-commands.ts`), where Playwright's client waits on `navigated` and `loadstate` events.
 *   `waitForURL` takes a glob or a `RegExp`, not a predicate function, and neither takes
 *   `networkidle`. A failed navigation does not reject `waitForURL`, which waits on to its timeout.
 *   An unknown `waitUntil` is reported as `waitUntil`, where Playwright names it `state` when the
 *   URL already matches.
 * - `allTextContents` sends `client/locator.ts` source text; compiled Playwright clients may
 *   format it differently. Like every `$$eval`, it runs in the page's main world, as Playwright's.
 */

/** Playwright's `WaitForFunctionOptions`: `polling` is `'raf'` (default) or an interval in ms. */
/** Playwright's `addScriptTag` / `addStyleTag` options. */
export type TagOptions = { url?: string; path?: string; content?: string }

/** The tags' protocol fields, in Playwright's schema order (`protocol.yml` `Frame.addScriptTag`,
 * `Frame.addStyleTag`). */
const TAG_KEYS = {
  script: ['url', 'content', 'type'],
  style: ['url', 'content'],
} as const

export type WaitForFunctionOptions = {
  polling?: number | 'raf'
  timeout?: number
  signal?: unknown
}

import type {
  FrameAriaSnapshotParams,
  FrameAriaSnapshotResult,
  FrameCheckParams,
  FrameClickParams,
  FrameFillParams,
  FrameGetAttributeParams,
  FrameGetAttributeResult,
  FrameGotoParams,
  FrameInitializer,
  FrameInnerTextParams,
  FrameInnerTextResult,
  FrameInputValueParams,
  FrameIsCheckedParams,
  FrameIsVisibleParams,
  FramePressParams,
  FrameQueryCountParams,
  FrameSelectOptionParams,
  FrameTextContentParams,
  FrameTextContentResult,
  FrameTitleResult,
  FrameUncheckParams,
  FrameWaitForSelectorParams,
  LifecycleEvent,
} from '@browseros/neo-runtime-spec/serve'
import type { ByRoleOptions } from '../vendor/playwright/isomorphic/locatorUtils'
import {
  getByAltTextSelector,
  getByLabelSelector,
  getByPlaceholderSelector,
  getByRoleSelector,
  getByTestIdSelector,
  getByTextSelector,
  getByTitleSelector,
} from '../vendor/playwright/isomorphic/locatorUtils'
import type { ApiName, Connection } from './connection'
import { ElementHandle, elementsFrom } from './element-handle'
import { FrameLocator, kAnyFrameSelector } from './frame-locator'
import {
  assertEvaluateOptions,
  assertMaxArguments,
  type CreatedHandle,
  type EvaluateOptions,
  evaluateParams,
  handleFrom,
  type JSHandle,
  type PageElement,
  type PageFunction,
  type PageFunctionOn,
  parseResult,
  serializeArgument,
} from './js-handle'
import { Locator, type LocatorOptions, testIdAttributeName } from './locator'
import { serializeUrlMatch } from './neo-commands'
import { type Response, responseFor } from './network'
import type { Page } from './page'
import {
  FRAME_ARIA_SNAPSHOT_KEYS,
  FRAME_CHECK_KEYS,
  FRAME_CLICK_KEYS,
  FRAME_FILL_KEYS,
  FRAME_GET_ATTRIBUTE_KEYS,
  FRAME_GOTO_KEYS,
  FRAME_INNER_TEXT_KEYS,
  FRAME_INPUT_VALUE_KEYS,
  FRAME_IS_CHECKED_KEYS,
  FRAME_IS_VISIBLE_KEYS,
  FRAME_PRESS_KEYS,
  FRAME_QUERY_COUNT_KEYS,
  FRAME_SELECT_OPTION_KEYS,
  FRAME_TEXT_CONTENT_KEYS,
  FRAME_UNCHECK_KEYS,
  FRAME_WAIT_FOR_SELECTOR_KEYS,
  pickParams,
} from './protocol'
import { get } from './runtime-bridge'
import {
  convertSelectOptionValues,
  type SelectOptionValues,
} from './select-option'

/** `click`'s options as a script passes them: the wire params plus `timeout`. */
export type ClickOptions = Omit<FrameClickParams, 'selector'> & {
  timeout?: number
}

/** `goto`'s options as a script passes them: the wire params plus `timeout`. */
export type GotoOptions = Omit<FrameGotoParams, 'url'> & { timeout?: number }

/** `textContent`'s options as a script passes them. */
export type TextContentOptions = Omit<FrameTextContentParams, 'selector'> & {
  timeout?: number
}

/** `innerText`'s options as a script passes them. */
export type InnerTextOptions = Omit<FrameInnerTextParams, 'selector'> & {
  timeout?: number
}

/** `getAttribute`'s options as a script passes them. */
export type GetAttributeOptions = Omit<
  FrameGetAttributeParams,
  'selector' | 'name'
> & {
  timeout?: number
}

/** Options shared by Playwright's page and locator snapshots. */
export type AriaSnapshotOptions = {
  mode?: 'ai' | 'default'
  depth?: number
  boxes?: boolean
  timeout?: number
}

/** `Frame.waitForSelector`'s options as `locator.waitFor` takes them (Playwright's
 * `channels.FrameWaitForSelectorOptions & TimeoutOptions`): `state` and `timeout`, plus `strict`
 * and `omitReturnValue`, which Playwright's client also passes through. */
export type WaitForSelectorOptions = Omit<
  FrameWaitForSelectorParams,
  'selector'
> & { timeout?: number }

/** What `waitForURL` matches: a glob string or a `RegExp`, two of Playwright's four `URLMatch`
 * forms. A predicate function is refused (`neo-commands.ts`); QuickJS has no `URLPattern`. */
export type URLMatch = string | RegExp

/** `waitForURL`'s options, as Playwright's. */
export type WaitForURLOptions = { waitUntil?: LifecycleEvent; timeout?: number }

/** `waitForLoadState`'s options, as Playwright's `TimeoutOptions`. */
export type WaitForLoadStateOptions = { timeout?: number }

/** Playwright's `kLifecycleEvents` (`client/types.ts`). */
const LIFECYCLE_EVENTS: ReadonlySet<string> = new Set([
  'load',
  'domcontentloaded',
  'networkidle',
  'commit',
])

/**
 * Playwright's `verifyLoadState` (`client/frame.ts`): `networkidle0` is an alias, and any other
 * unknown value throws a plain `Error` before a message is sent, so it has no `page.goto: `
 * prefix.
 */
export function verifyLoadState(
  name: string,
  waitUntil: LifecycleEvent,
): LifecycleEvent {
  if ((waitUntil as unknown) === 'networkidle0') waitUntil = 'networkidle'
  if (!LIFECYCLE_EVENTS.has(waitUntil))
    throw new Error(
      `${name}: expected one of (load|domcontentloaded|networkidle|commit)`,
    )
  return waitUntil
}

/**
 * {@link verifyLoadState} as `waitForLoadState` and `waitForURL` run it: inside Playwright's
 * `@apiCall` wrapper (`client/channelOwner.ts` `_wrapApiCall`), which prefixes its error with the
 * apiName, unlike `goto`'s.
 */
function verifyWaitState(
  apiName: ApiName,
  name: string,
  state: LifecycleEvent,
): LifecycleEvent {
  try {
    return verifyLoadState(name, state)
  } catch (error) {
    if (error instanceof Error) error.message = `${apiName}: ${error.message}`
    throw error
  }
}

export type FillOptions = Omit<FrameFillParams, 'selector' | 'value'> & {
  timeout?: number
}
export type PressOptions = Omit<FramePressParams, 'selector' | 'key'> & {
  timeout?: number
}
export type InputValueOptions = Omit<FrameInputValueParams, 'selector'> & {
  timeout?: number
}
export type IsCheckedOptions = Omit<FrameIsCheckedParams, 'selector'> & {
  timeout?: number
}
export type IsVisibleOptions = Omit<FrameIsVisibleParams, 'selector'> & {
  timeout?: number
}
export type CheckOptions = Omit<FrameCheckParams, 'selector'> & {
  timeout?: number
}
export type UncheckOptions = Omit<FrameUncheckParams, 'selector'> & {
  timeout?: number
}
export type SelectOptionOptions = Omit<
  FrameSelectOptionParams,
  'selector' | 'elements' | 'options'
> & { timeout?: number }

export class Frame {
  /** The opaque guid neo-runtime routes this frame's browser calls by. */
  readonly _guid: string
  readonly _page: Page
  readonly _connection: Connection
  #lastUrl: string
  #lastName: string

  constructor(
    guid: string,
    page: Page,
    connection: Connection,
    initializer: Pick<FrameInitializer, 'url' | 'name'> = { url: '', name: '' },
  ) {
    this._guid = guid
    this._page = page
    this._connection = connection
    this.#lastUrl = initializer.url
    this.#lastName = initializer.name
  }

  /** Refreshes values observed through frame records without changing this object's identity. */
  _updateInitializer(
    initializer: Pick<FrameInitializer, 'url' | 'name'>,
  ): void {
    this.#lastUrl = initializer.url
    this.#lastName = initializer.name
  }

  page(): Page {
    return this._page
  }

  url(): string {
    const url = get(this._guid, 'url')
    if (url !== null) this.#lastUrl = url
    return this.#lastUrl
  }

  name(): string {
    const name = get(this._guid, 'name')
    if (name !== null) this.#lastName = name
    return this.#lastName
  }

  parentFrame(): Frame | null {
    const record = get(this._guid, 'parentFrame')
    return record ? this._page._frameFrom(record) : null
  }

  childFrames(): Frame[] {
    return get(this._guid, 'childFrames').map((record) =>
      this._page._frameFrom(record),
    )
  }

  isDetached(): boolean {
    return get(this._guid, 'isDetached')
  }

  async goto(url: string, options: GotoOptions = {}): Promise<Response | null> {
    return await this._goto('frame.goto', url, options)
  }

  /** Sends `Frame.goto` under the navigation timeout, `waitUntil` defaulting to `load`, and
   * resolves the navigation's `Response`, built once per guid. */
  async _goto(
    apiName: ApiName,
    url: string,
    options: GotoOptions,
  ): Promise<Response | null> {
    const waitUntil = verifyLoadState(
      'waitUntil',
      options.waitUntil === undefined ? 'load' : options.waitUntil,
    )
    const result = await this._connection.send(
      apiName,
      this._guid,
      'goto',
      pickParams(FRAME_GOTO_KEYS, {
        url,
        ...options,
        waitUntil,
      }) as FrameGotoParams,
      this._page._timeoutSettings.navigationTimeout(options),
    )
    const guid = result?.response?.guid
    return guid ? responseFor(this._connection, guid, this) : null
  }

  /**
   * Waits until the frame's URL matches `url`, then until `waitUntil` (default `load`) fires, and
   * resolves `undefined`: at once if the current document already fired it, else on the next one
   * fired, as Playwright's does. A URL that already matches goes straight to `waitUntil`;
   * otherwise the next navigation that matches counts, a same-document one (a hash change,
   * `history.pushState`) included. Runs under the navigation timeout.
   *
   * `url` is a glob string, where `**` crosses `/` and `{a,b}` alternates (`''` matches every
   * URL), or a `RegExp`, flags included, tested as Playwright tests it. `waitUntil` is `load`,
   * `domcontentloaded` or `commit`. Neo limits: a predicate function and `networkidle` throw an
   * `Error`, and a failed navigation does not reject the wait.
   */
  async waitForURL(
    url: URLMatch,
    options: WaitForURLOptions = {},
  ): Promise<void> {
    return await this._waitForURL('frame.waitForURL', url, options)
  }

  /**
   * Sends `neo:waitForURL` under the navigation timeout. The url is checked first, then
   * `waitUntil`, and either throws before anything is sent, as Playwright's client does.
   */
  async _waitForURL(
    apiName: ApiName,
    url: URLMatch,
    options: WaitForURLOptions | undefined,
  ): Promise<void> {
    const waitOptions = options ?? {}
    const serialized = serializeUrlMatch(apiName, url)
    const waitUntil = verifyWaitState(
      apiName,
      'waitUntil',
      waitOptions.waitUntil === undefined ? 'load' : waitOptions.waitUntil,
    )
    await this._connection.sendNeo(
      apiName,
      this._guid,
      'neo:waitForURL',
      { url: serialized, waitUntil },
      this._page._timeoutSettings.navigationTimeout(waitOptions),
    )
  }

  /**
   * Waits until `state` (default `load`) has fired for the frame's current document, at once if
   * it already has, and resolves `undefined`; `commit` always has. Otherwise the next document's
   * `state` counts. Runs under the navigation timeout, and rejects when the page closes.
   * `state` is `load`, `domcontentloaded` or `commit`; `networkidle` is a Neo limit and throws an
   * `Error`.
   */
  async waitForLoadState(
    state?: LifecycleEvent,
    options?: WaitForLoadStateOptions,
  ): Promise<void> {
    return await this._waitForLoadState(
      'frame.waitForLoadState',
      state,
      options,
    )
  }

  /** Sends `neo:waitForLoadState` under the navigation timeout; an unknown `state` throws before
   * anything is sent. As Playwright's default parameter, only `undefined` means `load`. */
  async _waitForLoadState(
    apiName: ApiName,
    state: LifecycleEvent | undefined,
    options: WaitForLoadStateOptions | undefined,
  ): Promise<void> {
    const verified = verifyWaitState(
      apiName,
      'state',
      state === undefined ? 'load' : state,
    )
    await this._connection.sendNeo(
      apiName,
      this._guid,
      'neo:waitForLoadState',
      { state: verified },
      this._page._timeoutSettings.navigationTimeout(options ?? {}),
    )
  }

  /**
   * Sends `Frame.waitForSelector` under the action timeout. Behind `locator.waitFor` only, which
   * passes `omitReturnValue: true`: Neo has no `ElementHandle`s, so a public
   * `frame.waitForSelector` would have nothing to return.
   */
  async _waitForSelector(
    apiName: ApiName,
    selector: string,
    options: WaitForSelectorOptions,
  ): Promise<void> {
    await this._connection.send(
      apiName,
      this._guid,
      'waitForSelector',
      pickParams(FRAME_WAIT_FOR_SELECTOR_KEYS, {
        selector,
        ...options,
      }) as FrameWaitForSelectorParams,
      this._page._timeoutSettings.timeout(options),
    )
  }

  /**
   * Playwright's `frame.evaluate`: runs `pageFunction` (or an expression string) in this frame's
   * main world with `arg`, and resolves its JSON-serializable result (`js-handle.ts`).
   */
  async evaluate<R, Arg>(
    pageFunction: PageFunction<Arg, R>,
    arg?: Arg,
    options?: EvaluateOptions,
  ): Promise<R> {
    // biome-ignore lint/complexity/noArguments: Playwright counts the arguments the call passed
    assertMaxArguments(arguments.length, 3)
    return await this._evaluate('frame.evaluate', pageFunction, arg, options)
  }

  /** Sends `Frame.evaluateExpression` with no timeout, as Playwright's `kNoTimeout`. */
  async _evaluate<R>(
    apiName: ApiName,
    pageFunction: unknown,
    arg: unknown,
    options: EvaluateOptions | undefined,
  ): Promise<R> {
    assertEvaluateOptions(options)
    const params = evaluateParams(apiName, pageFunction, arg, options)
    const result = await this._connection.sendEval(
      apiName,
      this._guid,
      'evaluateExpression',
      params,
      0,
    )
    return parseResult(result?.value) as R
  }

  /** Playwright's `frame.evaluateHandle`: as `evaluate`, resolving a handle to the result. */
  async evaluateHandle<R, Arg>(
    pageFunction: PageFunction<Arg, R>,
    arg?: Arg,
    options?: EvaluateOptions,
  ): Promise<JSHandle<R>> {
    // biome-ignore lint/complexity/noArguments: Playwright counts the arguments the call passed
    assertMaxArguments(arguments.length, 3)
    return await this._evaluateHandle(
      'frame.evaluateHandle',
      pageFunction,
      arg,
      options,
    )
  }

  /** Sends `Frame.evaluateExpressionHandle` with no timeout. */
  async _evaluateHandle<R>(
    apiName: ApiName,
    pageFunction: unknown,
    arg: unknown,
    options: EvaluateOptions | undefined,
  ): Promise<JSHandle<R>> {
    assertEvaluateOptions(options)
    const params = evaluateParams(apiName, pageFunction, arg, options)
    const result = await this._connection.sendEval(
      apiName,
      this._guid,
      'evaluateExpressionHandle',
      params,
      0,
    )
    return handleFrom(this._connection, result.handle) as JSHandle<R>
  }

  /**
   * Playwright's `Locator._withElement` (`client/locator.ts`): one strict `waitForSelector` for
   * an attached element under the action timeout, `task` on the `ElementHandle` Rust answers,
   * then `dispose`, whatever the task did.
   */
  async _withElement<R>(
    apiName: ApiName,
    selector: string,
    task: (handle: ElementHandle) => Promise<R>,
    options: { timeout?: number; signal?: unknown },
  ): Promise<R> {
    // `null` is no signal, as `undefined`.
    if (options.signal !== undefined && options.signal !== null)
      // Deviation: `client/timeoutSettings.ts` `TimeoutSettings.timeout`: `return { signal: options.signal, timeout: this._timeout(options) };` Neo refuses a signal rather than ignore it.
      throw new Error(`${apiName}: Neo does not support signal yet`)
    const result = (await this._connection.send(
      apiName,
      this._guid,
      'waitForSelector',
      { selector, strict: true, state: 'attached' },
      this._page._timeoutSettings.timeout(options),
    )) as { element?: CreatedHandle } | undefined
    const handle = ElementHandle.fromNullable(this._connection, result?.element)
    if (!handle) throw new Error(`Could not resolve ${selector} to DOM Element`)
    try {
      return await task(handle)
    } finally {
      await handle._dispose(apiName)
    }
  }

  async title(): Promise<string> {
    return await this._title('frame.title')
  }

  /** Sends `Frame.title` with no timeout, as Playwright's `kNoTimeout`. */
  async _title(apiName: ApiName): Promise<string> {
    const result = (await this._connection.send(
      apiName,
      this._guid,
      'title',
      {},
      0,
    )) as FrameTitleResult
    return result.value
  }

  /** Not strict, as Playwright's `frame.textContent`. */
  async textContent(
    selector: string,
    options: TextContentOptions = {},
  ): Promise<string | null> {
    return await this._textContent('frame.textContent', selector, options)
  }

  async _textContent(
    apiName: ApiName,
    selector: string,
    options: TextContentOptions,
  ): Promise<string | null> {
    const result = (await this._connection.send(
      apiName,
      this._guid,
      'textContent',
      pickParams(FRAME_TEXT_CONTENT_KEYS, {
        selector,
        ...options,
      }) as FrameTextContentParams,
      this._page._timeoutSettings.timeout(options ?? {}),
    )) as FrameTextContentResult
    return result.value === undefined ? null : result.value
  }

  async innerText(
    selector: string,
    options: InnerTextOptions = {},
  ): Promise<string> {
    return await this._innerText('frame.innerText', selector, options)
  }

  async _innerText(
    apiName: ApiName,
    selector: string,
    options: InnerTextOptions,
  ): Promise<string> {
    const result = (await this._connection.send(
      apiName,
      this._guid,
      'innerText',
      pickParams(FRAME_INNER_TEXT_KEYS, {
        selector,
        ...options,
      }) as FrameInnerTextParams,
      this._page._timeoutSettings.timeout(options ?? {}),
    )) as FrameInnerTextResult
    return result.value
  }

  async getAttribute(
    selector: string,
    name: string,
    options: GetAttributeOptions = {},
  ): Promise<string | null> {
    return await this._getAttribute(
      'frame.getAttribute',
      selector,
      name,
      options,
    )
  }

  async _getAttribute(
    apiName: ApiName,
    selector: string,
    name: string,
    options: GetAttributeOptions,
  ): Promise<string | null> {
    const result = (await this._connection.send(
      apiName,
      this._guid,
      'getAttribute',
      pickParams(FRAME_GET_ATTRIBUTE_KEYS, {
        selector,
        name,
        ...options,
      }) as FrameGetAttributeParams,
      this._page._timeoutSettings.timeout(options ?? {}),
    )) as FrameGetAttributeResult
    return result.value === undefined ? null : result.value
  }

  /** Playwright's `Frame._queryCount`: a non-strict snapshot with `kNoTimeout`. */
  async _queryCount(apiName: ApiName, selector: string): Promise<number> {
    const result = await this._connection.send(
      apiName,
      this._guid,
      'queryCount',
      pickParams(FRAME_QUERY_COUNT_KEYS, { selector }) as FrameQueryCountParams,
      0,
    )
    if (result === undefined)
      throw new Error(
        'neo-sdk: Frame.queryCount contract error: missing result',
      )
    return result.value
  }

  /** The source text Neo's SDK sends, from `client/locator.ts` at 26137face. */
  async _allTextContents(
    apiName: ApiName,
    selector: string,
  ): Promise<string[]> {
    // Playwright's `allTextContents` is this `$$eval` of its own function, in the main world.
    const result = await this._connection.sendEval(
      apiName,
      this._guid,
      'evalOnSelectorAll',
      {
        selector,
        expression: "ee => ee.map(e => e.textContent || '')",
        isFunction: true,
        arg: serializeArgument(undefined),
      },
      0,
    )
    // Playwright answers whatever the page's function returned (a `textContent` getter may give
    // a number); Neo does not narrow it.
    return parseResult(result?.value) as string[]
  }

  /** Playwright's `frame.addScriptTag`: a `<script>` from `url` or `content` in the page's head,
   * as an `ElementHandle` once it loaded (or ran). */
  async addScriptTag(
    options: TagOptions & { type?: string } = {},
  ): Promise<ElementHandle> {
    return await this._addTag('frame.addScriptTag', 'addScriptTag', options)
  }

  /** Playwright's `frame.addStyleTag`: a `<style>` or stylesheet `<link>`, once it loaded. */
  async addStyleTag(options: TagOptions = {}): Promise<ElementHandle> {
    return await this._addTag('frame.addStyleTag', 'addStyleTag', options)
  }

  /** Both tags: Playwright's client reads `path` into `content`; Neo reads files only from the
   * run's artifact directory, which a run does not have yet. */
  async _addTag(
    apiName: ApiName,
    method: 'addScriptTag' | 'addStyleTag',
    options: TagOptions & { type?: string },
  ): Promise<ElementHandle> {
    if (options.path)
      // Deviation: `client/frame.ts` `Frame.addScriptTag`: `copy.content = (await fs.promises.readFile(copy.path)).toString();` Neo reads files only from the run's artifact directory.
      throw new Error(
        `${apiName}: path: this run has no artifact directory, so Neo cannot read "${options.path}"`,
      )
    // Playwright's validator refuses a `null` field (`tOptional(tString)`) inside the call, so
    // its message names the call; `path` and a style's `type` are no protocol field, so it
    // drops them.
    let params: { url?: string; content?: string; type?: string }
    try {
      params = pickParams(
        method === 'addScriptTag' ? TAG_KEYS.script : TAG_KEYS.style,
        options,
      ) as typeof params
    } catch (error) {
      if (error instanceof Error) error.message = `${apiName}: ${error.message}`
      throw error
    }
    const result = await this._connection.sendEval(
      apiName,
      this._guid,
      method,
      params,
      0,
    )
    const handle = ElementHandle.fromNullable(this._connection, result.element)
    if (!handle)
      throw new Error(`neo-sdk: Frame.${method} contract error: no element`)
    return handle
  }

  /** Playwright's `frame.$`: the first match, or `null`, as an `ElementHandle`. */
  async $(
    selector: string,
    options?: { strict?: boolean },
  ): Promise<ElementHandle | null> {
    return await this._querySelector('frame.$', selector, options)
  }

  async _querySelector(
    apiName: ApiName,
    selector: string,
    options: { strict?: boolean; signal?: unknown } | undefined,
  ): Promise<ElementHandle | null> {
    // `null` is no signal, as `undefined`.
    if (options?.signal !== undefined && options.signal !== null)
      // Deviation: `client/timeoutSettings.ts` `kNoTimeout`: `export const kNoTimeout: channels.TimeoutOptions = { signal: undefined, timeout: 0 };` Playwright's `$` sends this, so it ignores a caller's signal; Neo refuses one rather than ignore it.
      throw new Error(`${apiName}: Neo does not support signal yet`)
    // Playwright's validator lets only `undefined` through: `strict: null` rejects before the call.
    const params = {
      selector,
      ...pickParams(['strict'] as const, options ?? {}),
    } as { selector: string; strict?: boolean }
    const result = await this._connection.sendEval(
      apiName,
      this._guid,
      'querySelector',
      params,
      0,
    )
    return ElementHandle.fromNullable(this._connection, result?.element)
  }

  /** Playwright's `frame.$$`: every match, as `ElementHandle`s. */
  async $$(selector: string): Promise<ElementHandle[]> {
    return await this._querySelectorAll('frame.$$', selector)
  }

  async _querySelectorAll(
    apiName: ApiName,
    selector: string,
  ): Promise<ElementHandle[]> {
    const result = await this._connection.sendEval(
      apiName,
      this._guid,
      'querySelectorAll',
      { selector },
      0,
    )
    return elementsFrom(this._connection, result?.elements ?? [])
  }

  /** Playwright's `frame.$eval`: `pageFunction` on the first match, by value. */
  async $eval<R, Arg>(
    selector: string,
    pageFunction: PageFunctionOn<PageElement, Arg, R>,
    arg?: Arg,
  ): Promise<R> {
    // biome-ignore lint/complexity/noArguments: Playwright counts the arguments the call passed
    assertMaxArguments(arguments.length, 3)
    return await this._evalOnSelector(
      'frame.$eval',
      selector,
      pageFunction,
      arg,
    )
  }

  async _evalOnSelector<R>(
    apiName: ApiName,
    selector: string,
    pageFunction: unknown,
    arg: unknown,
  ): Promise<R> {
    const params = evaluateParams(apiName, pageFunction, arg, undefined)
    const result = await this._connection.sendEval(
      apiName,
      this._guid,
      'evalOnSelector',
      { selector, ...params },
      0,
    )
    return parseResult(result?.value) as R
  }

  /** Playwright's `frame.$$eval`: `pageFunction` on the array of every match, by value. */
  async $$eval<R, Arg>(
    selector: string,
    pageFunction: PageFunctionOn<PageElement[], Arg, R>,
    arg?: Arg,
  ): Promise<R> {
    // biome-ignore lint/complexity/noArguments: Playwright counts the arguments the call passed
    assertMaxArguments(arguments.length, 3)
    return await this._evalOnSelectorAll(
      'frame.$$eval',
      selector,
      pageFunction,
      arg,
    )
  }

  async _evalOnSelectorAll<R>(
    apiName: ApiName,
    selector: string,
    pageFunction: unknown,
    arg: unknown,
  ): Promise<R> {
    const params = evaluateParams(apiName, pageFunction, arg, undefined)
    const result = await this._connection.sendEval(
      apiName,
      this._guid,
      'evalOnSelectorAll',
      { selector, ...params },
      0,
    )
    return parseResult(result?.value) as R
  }

  /**
   * Playwright's `frame.waitForFunction`: polls `pageFunction` in the page (every animation
   * frame, or every `polling` ms) until it returns a truthy value, under the action timeout, and
   * resolves a handle to that value.
   */
  async waitForFunction<R, Arg>(
    pageFunction: PageFunction<Arg, R>,
    arg?: Arg,
    options?: WaitForFunctionOptions,
  ): Promise<JSHandle<R>> {
    return await this._waitForFunction(
      'frame.waitForFunction',
      pageFunction,
      arg,
      options,
    )
  }

  async _waitForFunction<R>(
    apiName: ApiName,
    pageFunction: unknown,
    arg: unknown,
    options: WaitForFunctionOptions = {},
  ): Promise<JSHandle<R>> {
    // Playwright: `client/frame.ts` `Frame.waitForFunction`: `assert(options.polling === 'raf', 'Unknown polling option: ' + options.polling);`
    if (typeof options.polling === 'string' && options.polling !== 'raf')
      throw new Error(`Unknown polling option: ${options.polling}`)
    // `null` is no signal, as `undefined`.
    if (options.signal !== undefined && options.signal !== null)
      // Deviation: `client/timeoutSettings.ts` `TimeoutSettings.timeout`: `return { signal: options.signal, timeout: this._timeout(options) };` Neo refuses a signal rather than ignore it.
      throw new Error(`${apiName}: Neo does not support signal yet`)
    const params = evaluateParams(apiName, pageFunction, arg, undefined)
    // Playwright's validator lets only `undefined` through: `polling: null` rejects before the call.
    const { pollingInterval } = pickParams(['pollingInterval'] as const, {
      pollingInterval: options.polling === 'raf' ? undefined : options.polling,
    }) as { pollingInterval?: number }
    const result = await this._connection.sendEval(
      apiName,
      this._guid,
      'waitForFunction',
      pollingInterval === undefined ? params : { ...params, pollingInterval },
      this._page._timeoutSettings.timeout(options),
    )
    return handleFrom(this._connection, result.handle) as JSHandle<R>
  }

  /**
   * `Frame.waitForSelector` answering its element, as Playwright's `frame.waitForSelector` does
   * for an attached or visible wait: behind `locator.elementHandle`.
   */
  async _waitForSelectorElement(
    apiName: ApiName,
    selector: string,
    options: WaitForSelectorOptions & { strict?: boolean; signal?: unknown },
  ): Promise<ElementHandle | null> {
    // `null` is no signal, as `undefined`.
    if (options.signal !== undefined && options.signal !== null)
      // Deviation: `client/timeoutSettings.ts` `TimeoutSettings.timeout`: `return { signal: options.signal, timeout: this._timeout(options) };` Neo refuses a signal rather than ignore it.
      throw new Error(`${apiName}: Neo does not support signal yet`)
    const result = (await this._connection.send(
      apiName,
      this._guid,
      'waitForSelector',
      pickParams(FRAME_WAIT_FOR_SELECTOR_KEYS, {
        selector,
        ...options,
      }) as FrameWaitForSelectorParams,
      this._page._timeoutSettings.timeout(options),
    )) as { element?: CreatedHandle } | undefined
    return ElementHandle.fromNullable(this._connection, result?.element)
  }

  /** Playwright: `client/locator.ts` `Locator.ariaSnapshot`: `selector: this._selector,` */
  async _ariaSnapshot(
    apiName: ApiName,
    options: AriaSnapshotOptions,
    selector?: string,
  ): Promise<string> {
    const result = (await this._connection.send(
      apiName,
      this._guid,
      'ariaSnapshot',
      pickParams(FRAME_ARIA_SNAPSHOT_KEYS, {
        mode: options.mode,
        selector,
        depth: options.depth,
        boxes: options.boxes,
      }) as FrameAriaSnapshotParams,
      this._page._timeoutSettings.timeout(options),
    )) as FrameAriaSnapshotResult
    return result.snapshot
  }

  /** Not strict, as Playwright's `frame.click`. */
  async click(selector: string, options: ClickOptions = {}): Promise<void> {
    return await this._click('frame.click', selector, options)
  }

  /** Sends `Frame.click`. */
  async _click(
    apiName: ApiName,
    selector: string,
    options: ClickOptions,
  ): Promise<void> {
    await this._connection.send(
      apiName,
      this._guid,
      'click',
      pickParams(FRAME_CLICK_KEYS, {
        selector,
        ...options,
      }) as FrameClickParams,
      this._page._timeoutSettings.timeout(options ?? {}),
    )
  }

  async fill(
    selector: string,
    value: string,
    options: FillOptions = {},
  ): Promise<void> {
    return await this._fill('frame.fill', selector, value, options)
  }

  /** Synchronously reaches `send` so it captures the script's call site. */
  async _fill(
    apiName: ApiName,
    selector: string,
    value: string,
    options: FillOptions,
  ): Promise<void> {
    await this._connection.send(
      apiName,
      this._guid,
      'fill',
      pickParams(FRAME_FILL_KEYS, {
        selector,
        value,
        ...options,
      }) as FrameFillParams,
      this._page._timeoutSettings.timeout(options ?? {}),
    )
  }

  async press(
    selector: string,
    key: string,
    options: PressOptions = {},
  ): Promise<void> {
    return await this._press('frame.press', selector, key, options)
  }

  /** Synchronously reaches `send` so it captures the script's call site. */
  async _press(
    apiName: ApiName,
    selector: string,
    key: string,
    options: PressOptions,
  ): Promise<void> {
    await this._connection.send(
      apiName,
      this._guid,
      'press',
      pickParams(FRAME_PRESS_KEYS, {
        selector,
        key,
        ...options,
      }) as FramePressParams,
      this._page._timeoutSettings.timeout(options ?? {}),
    )
  }

  async inputValue(
    selector: string,
    options: InputValueOptions = {},
  ): Promise<string> {
    return await this._inputValue('frame.inputValue', selector, options)
  }

  /** Synchronously reaches `send` so it captures the script's call site. */
  async _inputValue(
    apiName: ApiName,
    selector: string,
    options: InputValueOptions,
  ): Promise<string> {
    const result = await this._connection.send(
      apiName,
      this._guid,
      'inputValue',
      pickParams(FRAME_INPUT_VALUE_KEYS, {
        selector,
        ...options,
      }) as FrameInputValueParams,
      this._page._timeoutSettings.timeout(options ?? {}),
    )
    if (result === undefined)
      throw new Error(
        'neo-sdk: Frame.inputValue contract error: missing result',
      )
    return result.value
  }

  async isVisible(
    selector: string,
    options: IsVisibleOptions = {},
  ): Promise<boolean> {
    return await this._isVisible('frame.isVisible', selector, options)
  }

  /** Playwright's `kNoTimeout`: visibility reads once and ignores `options.timeout`. */
  async _isVisible(
    apiName: ApiName,
    selector: string,
    options: IsVisibleOptions,
  ): Promise<boolean> {
    const result = await this._connection.send(
      apiName,
      this._guid,
      'isVisible',
      pickParams(FRAME_IS_VISIBLE_KEYS, {
        selector,
        ...options,
      }) as FrameIsVisibleParams,
      0,
    )
    if (result === undefined)
      throw new Error('neo-sdk: Frame.isVisible contract error: missing result')
    return result.value
  }

  async isChecked(
    selector: string,
    options: IsCheckedOptions = {},
  ): Promise<boolean> {
    return await this._isChecked('frame.isChecked', selector, options)
  }

  /** Checked reads wait for the first match under the caller's action timeout. */
  async _isChecked(
    apiName: ApiName,
    selector: string,
    options: IsCheckedOptions,
  ): Promise<boolean> {
    const result = await this._connection.send(
      apiName,
      this._guid,
      'isChecked',
      pickParams(FRAME_IS_CHECKED_KEYS, {
        selector,
        ...options,
      }) as FrameIsCheckedParams,
      this._page._timeoutSettings.timeout(options ?? {}),
    )
    if (result === undefined)
      throw new Error('neo-sdk: Frame.isChecked contract error: missing result')
    return result.value
  }

  async check(selector: string, options: CheckOptions = {}): Promise<void> {
    return await this._check('frame.check', selector, options)
  }

  /** Synchronously reaches `send` so it captures the script's call site. */
  async _check(
    apiName: ApiName,
    selector: string,
    options: CheckOptions,
  ): Promise<void> {
    await this._connection.send(
      apiName,
      this._guid,
      'check',
      pickParams(FRAME_CHECK_KEYS, {
        selector,
        ...options,
      }) as FrameCheckParams,
      this._page._timeoutSettings.timeout(options ?? {}),
    )
  }

  async uncheck(selector: string, options: UncheckOptions = {}): Promise<void> {
    return await this._uncheck('frame.uncheck', selector, options)
  }

  /** Synchronously reaches `send` so it captures the script's call site. */
  async _uncheck(
    apiName: ApiName,
    selector: string,
    options: UncheckOptions,
  ): Promise<void> {
    await this._connection.send(
      apiName,
      this._guid,
      'uncheck',
      pickParams(FRAME_UNCHECK_KEYS, {
        selector,
        ...options,
      }) as FrameUncheckParams,
      this._page._timeoutSettings.timeout(options ?? {}),
    )
  }

  async selectOption(
    selector: string,
    values: SelectOptionValues,
    options: SelectOptionOptions = {},
  ): Promise<string[]> {
    return await this._selectOption(
      'frame.selectOption',
      selector,
      values,
      options,
    )
  }

  /** Synchronously reaches `send` so it captures the script's call site. */
  async _selectOption(
    apiName: ApiName,
    selector: string,
    values: SelectOptionValues,
    options: SelectOptionOptions,
  ): Promise<string[]> {
    const result = await this._connection.send(
      apiName,
      this._guid,
      'selectOption',
      pickParams(FRAME_SELECT_OPTION_KEYS, {
        selector,
        ...convertSelectOptionValues(values),
        ...options,
      }) as FrameSelectOptionParams,
      this._page._timeoutSettings.timeout(options ?? {}),
    )
    if (result === undefined)
      throw new Error(
        'neo-sdk: Frame.selectOption contract error: missing result',
      )
    return result.values
  }

  async setChecked(
    selector: string,
    checked: boolean,
    options: CheckOptions = {},
  ): Promise<void> {
    if (checked) await this._check('frame.setChecked', selector, options)
    else await this._uncheck('frame.setChecked', selector, options)
  }

  locator(selector: string, options?: LocatorOptions): Locator {
    return new Locator(this, selector, options)
  }

  frameLocator(selector?: string): FrameLocator {
    return new FrameLocator(
      this,
      selector === undefined ? kAnyFrameSelector : selector,
    )
  }

  getByTestId(testId: string | RegExp): Locator {
    return this.locator(getByTestIdSelector(testIdAttributeName(), testId))
  }

  getByAltText(text: string | RegExp, options?: { exact?: boolean }): Locator {
    return this.locator(getByAltTextSelector(text, options))
  }

  getByLabel(text: string | RegExp, options?: { exact?: boolean }): Locator {
    return this.locator(getByLabelSelector(text, options))
  }

  getByPlaceholder(
    text: string | RegExp,
    options?: { exact?: boolean },
  ): Locator {
    return this.locator(getByPlaceholderSelector(text, options))
  }

  getByText(text: string | RegExp, options?: { exact?: boolean }): Locator {
    return this.locator(getByTextSelector(text, options))
  }

  getByTitle(text: string | RegExp, options?: { exact?: boolean }): Locator {
    return this.locator(getByTitleSelector(text, options))
  }

  getByRole(role: string, options: ByRoleOptions = {}): Locator {
    return this.locator(getByRoleSelector(role, options))
  }

  /** Playwright's `ChannelOwner.toJSON`: the frame's type and guid only, also inside a
   * serialized locator. */
  toJSON(): { _type: 'Frame'; _guid: string } {
    return { _type: 'Frame', _guid: this._guid }
  }
}
