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
 * `goto` and `waitForNavigation` resolve the navigation's `Response` (`network.ts`), or `null`
 * where Playwright's does: a same-document change, or a document without a network request
 * (`about:blank`, `data:`).
 *
 * What differs from Playwright:
 * - Detached frames retain the last URL and name read by a getter or frame record, as Page
 *   retains its last URL after closure; without frame events, unseen changes are not retained.
 * - `waitForURL`, `waitForLoadState` and `waitForNavigation` are `neo:` calls that Rust waits out
 *   (`neo-commands.ts`), where Playwright's client waits on `navigated` and `loadstate` events.
 *   A predicate `url` steps a cursor, one call per navigation, and receives Neo's read-only `URL`
 *   (`url.ts`). None takes `networkidle`. A failed navigation rejects a main-frame wait, as in
 *   Playwright, once Neo has seen its request start; a child frame's never does.
 *   `waitForNavigation` on a child frame rejects: Neo does not record a child frame's requests.
 *   An unknown `waitUntil` is reported as `waitUntil`, where Playwright names it `state` when a
 *   glob or `RegExp` URL already matches.
 * - `allTextContents` sends `client/locator.ts` source text; compiled Playwright clients may
 *   format it differently. It reads in Neo's isolated world, so page-defined `textContent`
 *   overrides (prototype patches, custom element classes or per-instance properties) can differ.
 */

import type {
  FrameAriaSnapshotParams,
  FrameAriaSnapshotResult,
  FrameCheckParams,
  FrameClickParams,
  FrameDblclickParams,
  FrameFillParams,
  FrameGetAttributeParams,
  FrameGetAttributeResult,
  FrameGotoParams,
  FrameHoverParams,
  FrameInitializer,
  FrameInnerTextParams,
  FrameInnerTextResult,
  FrameInputValueParams,
  FrameIsCheckedParams,
  FrameIsVisibleParams,
  FramePressParams,
  FrameQueryCountParams,
  FrameSelectOptionParams,
  FrameTapParams,
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
import { TimeoutError } from './errors'
import { FrameLocator, kAnyFrameSelector } from './frame-locator'
import { Locator, type LocatorOptions, testIdAttributeName } from './locator'
import { type NeoNavigationResult, serializeUrlMatch } from './neo-commands'
import { type Response, responseFor } from './network'
import type { Page } from './page'
import {
  FRAME_ARIA_SNAPSHOT_KEYS,
  FRAME_CHECK_KEYS,
  FRAME_CLICK_KEYS,
  FRAME_DBLCLICK_KEYS,
  FRAME_FILL_KEYS,
  FRAME_GET_ATTRIBUTE_KEYS,
  FRAME_GOTO_KEYS,
  FRAME_HOVER_KEYS,
  FRAME_INNER_TEXT_KEYS,
  FRAME_INPUT_VALUE_KEYS,
  FRAME_IS_CHECKED_KEYS,
  FRAME_IS_VISIBLE_KEYS,
  FRAME_PRESS_KEYS,
  FRAME_QUERY_COUNT_KEYS,
  FRAME_SELECT_OPTION_KEYS,
  FRAME_TAP_KEYS,
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
import { type URLPredicate, urlFor } from './url'

/** `click`'s options as a script passes them: the wire params plus `timeout`. */
export type ClickOptions = Omit<FrameClickParams, 'selector'> & {
  timeout?: number
}

/**
 * `dblclick`'s options as a script passes them: the wire params plus `timeout`, and the
 * deprecated `noWaitAfter`, which Playwright's client accepts and its protocol drops.
 */
export type DblclickOptions = Omit<FrameDblclickParams, 'selector'> & {
  timeout?: number
  noWaitAfter?: boolean
}

/** `hover`'s options as a script passes them, with the deprecated `noWaitAfter` (dropped). */
export type HoverOptions = Omit<FrameHoverParams, 'selector'> & {
  timeout?: number
  noWaitAfter?: boolean
}

/** `tap`'s options as a script passes them, with the deprecated `noWaitAfter` (dropped). */
export type TapOptions = Omit<FrameTapParams, 'selector'> & {
  timeout?: number
  noWaitAfter?: boolean
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

/** What `page.frame({ url })` matches: a glob string or a `RegExp`, two of Playwright's four
 * `URLMatch` forms; a predicate function is refused there (`neo-commands.ts`). */
export type URLMatch = string | RegExp

/** What `waitForURL` and `waitForNavigation` match: Playwright's `URLMatch` without
 * `URLPattern`, which QuickJS lacks: a glob string, a `RegExp`, or a predicate on the navigated
 * `URL` (Neo's read-only one, `url.ts`). */
export type NavigationURLMatch = URLMatch | URLPredicate

/** `waitForURL`'s options, as Playwright's. */
export type WaitForURLOptions = { waitUntil?: LifecycleEvent; timeout?: number }

/** Playwright's `WaitForNavigationOptions` (`client/frame.ts`), without `signal`: Neo takes no
 * `AbortSignal` (`errors.ts`). */
export type WaitForNavigationOptions = {
  url?: NavigationURLMatch
  waitUntil?: LifecycleEvent
  timeout?: number
}

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
   * URL), a `RegExp`, flags included, tested as Playwright tests it, or a predicate that receives
   * the URL as a read-only `URL` and may be async. `waitUntil` is `load`, `domcontentloaded` or
   * `commit`. A navigation that fails before the URL matched rejects the wait. Neo limit:
   * `networkidle` throws an `Error`.
   */
  async waitForURL(
    url: NavigationURLMatch,
    options: WaitForURLOptions = {},
  ): Promise<void> {
    return await this._waitForURL('frame.waitForURL', url, options)
  }

  /**
   * Sends `neo:waitForURL` under the navigation timeout, or for a predicate checks the current
   * URL and then steps a navigation cursor (`_navigationMatching`). The url is checked first,
   * then `waitUntil`, and either throws before anything is sent, as Playwright's client does.
   */
  async _waitForURL(
    apiName: ApiName,
    url: NavigationURLMatch,
    options: WaitForURLOptions | undefined,
  ): Promise<void> {
    const waitOptions = options ?? {}
    if (typeof url === 'function') {
      // Playwright's client `waitForURL`: a current URL the predicate accepts (truthy, not
      // awaited, as `urlMatches` answers) waits for the load state; otherwise the next
      // navigation it accepts does, as `waitForNavigation({ url })`.
      const current = urlFor(this.url())
      if (current && url(current))
        return await this._waitForLoadState(
          apiName,
          waitOptions.waitUntil,
          waitOptions,
        )
      const waitUntil = verifyWaitState(
        apiName,
        'waitUntil',
        waitOptions.waitUntil === undefined ? 'load' : waitOptions.waitUntil,
      )
      await this._navigationMatching(
        apiName,
        url,
        waitUntil,
        this._page._timeoutSettings.navigationTimeout(waitOptions),
        false,
      )
      return
    }
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
   * Playwright's deprecated `waitForNavigation`: waits for the frame's next navigation after the
   * call whose URL matches `url` (any, by default; a glob, a `RegExp` or a predicate on its `URL`),
   * a same-document one (a hash change, `history.pushState`) included, then for `waitUntil`
   * (default `load`), and resolves the new document's `Response`, or `null` for a same-document
   * navigation or a document without a request. A navigation that fails first rejects with its
   * error. Start it before the action that navigates:
   * `await Promise.all([page.waitForNavigation(), page.click('a')])`. Runs under the navigation
   * timeout. Neo limits: `networkidle` throws, and a child frame's wait rejects, because Neo does
   * not record a child frame's navigation requests.
   */
  async waitForNavigation(
    options: WaitForNavigationOptions = {},
  ): Promise<Response | null> {
    return await this._waitForNavigation('frame.waitForNavigation', options)
  }

  /** Sends `neo:waitForNavigation` (glob, `RegExp` or no `url`) or steps a predicate's cursor,
   * under the navigation timeout. `waitUntil` and `url` are checked before anything is sent. */
  async _waitForNavigation(
    apiName: ApiName,
    options: WaitForNavigationOptions,
  ): Promise<Response | null> {
    const waitUntil = verifyWaitState(
      apiName,
      'waitUntil',
      options.waitUntil === undefined ? 'load' : options.waitUntil,
    )
    const timeout = this._page._timeoutSettings.navigationTimeout(options)
    const result =
      typeof options.url === 'function'
        ? await this._navigationMatching(
            apiName,
            options.url,
            waitUntil,
            timeout,
            true,
          )
        : await this._connection.sendNeo(
            apiName,
            this._guid,
            'neo:waitForNavigation',
            { url: serializeUrlMatch(apiName, options.url), waitUntil },
            timeout,
          )
    const guid = result?.response?.guid
    return guid ? responseFor(this._connection, guid, this) : null
  }

  /**
   * A URL predicate's wait (`neo-commands.ts`, DECIDED 3): Rust answers each navigation of this
   * frame in order, from a cursor subscribed by the first call; the predicate tests its `URL`,
   * awaited as Playwright's waiter awaits it (a URL the parser rejects is never accepted); the first
   * it accepts is settled for `waitUntil`. `respond` asks for that navigation's `Response`.
   * `timeout` (0 for none) bounds the whole loop: each call gets what is left, and a predicate
   * still pending at the deadline rejects the wait then, as Playwright's waiter does.
   */
  async _navigationMatching(
    apiName: ApiName,
    predicate: URLPredicate,
    waitUntil: LifecycleEvent,
    timeout: number,
    respond: boolean,
  ): Promise<NeoNavigationResult> {
    const deadline = timeout ? Date.now() + timeout : 0
    const left = () => {
      if (!deadline) return 0
      const ms = deadline - Date.now()
      if (ms <= 0) throw navigationTimeout(apiName, timeout)
      return ms
    }
    try {
      let cursor: number | undefined
      for (;;) {
        const next = await this._connection.sendNeo(
          apiName,
          this._guid,
          'neo:nextNavigation',
          cursor === undefined
            ? { response: respond }
            : { cursor, response: respond },
          left(),
        )
        cursor = next.cursor
        const url = urlFor(next.url)
        let verdict: boolean | Promise<boolean> = false
        try {
          if (url) verdict = predicate(url)
        } catch (error) {
          throw prefixed(apiName, error)
        }
        const accepted = await verdictBefore(verdict, deadline, () =>
          navigationTimeout(apiName, timeout),
        ).catch((error) => {
          throw error instanceof TimeoutError ? error : prefixed(apiName, error)
        })
        if (accepted)
          return await this._connection.sendNeo(
            apiName,
            this._guid,
            'neo:finishNavigation',
            { cursor, waitUntil },
            left(),
          )
      }
    } catch (error) {
      // Each step ran under what was left of `timeout`; its message names the whole timeout.
      if (error instanceof TimeoutError && deadline)
        error.message = error.message.replace(
          /Timeout [\d.]+ms exceeded\./,
          `Timeout ${timeout}ms exceeded.`,
        )
      throw error
    }
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
    const result = await this._connection.send(
      apiName,
      this._guid,
      'evalOnSelectorAll',
      {
        selector,
        expression: "ee => ee.map(e => e.textContent || '')",
        isFunction: true,
        arg: { value: { v: 'undefined' }, handles: [] },
      },
      0,
    )
    // `protocol/serializers.ts` assigns id 1 to the outer array and encodes each string as {s}.
    const value = result?.value
    if (
      !value ||
      typeof value !== 'object' ||
      Object.keys(value).length !== 2 ||
      value.id !== 1 ||
      !Array.isArray(value.a)
    )
      throw new Error(
        'neo-sdk: Frame.evalOnSelectorAll contract error: expected a serialized string array',
      )
    return value.a.map((item) => {
      if (
        !item ||
        typeof item !== 'object' ||
        Object.keys(item).length !== 1 ||
        typeof item.s !== 'string'
      )
        throw new Error(
          'neo-sdk: Frame.evalOnSelectorAll contract error: expected a serialized string',
        )
      return item.s
    })
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

  /** Not strict, as Playwright's `frame.dblclick`. */
  async dblclick(
    selector: string,
    options: DblclickOptions = {},
  ): Promise<void> {
    return await this._dblclick('frame.dblclick', selector, options)
  }

  /** Sends `Frame.dblclick`. */
  async _dblclick(
    apiName: ApiName,
    selector: string,
    options: DblclickOptions,
  ): Promise<void> {
    await this._connection.send(
      apiName,
      this._guid,
      'dblclick',
      pickParams(FRAME_DBLCLICK_KEYS, {
        selector,
        ...options,
      }) as FrameDblclickParams,
      this._page._timeoutSettings.timeout(options ?? {}),
    )
  }

  /** Not strict, as Playwright's `frame.hover`. */
  async hover(selector: string, options: HoverOptions = {}): Promise<void> {
    return await this._hover('frame.hover', selector, options)
  }

  /** Sends `Frame.hover`. */
  async _hover(
    apiName: ApiName,
    selector: string,
    options: HoverOptions,
  ): Promise<void> {
    await this._connection.send(
      apiName,
      this._guid,
      'hover',
      pickParams(FRAME_HOVER_KEYS, {
        selector,
        ...options,
      }) as FrameHoverParams,
      this._page._timeoutSettings.timeout(options ?? {}),
    )
  }

  /** Not strict, as Playwright's `frame.tap`. */
  async tap(selector: string, options: TapOptions = {}): Promise<void> {
    return await this._tap('frame.tap', selector, options)
  }

  /** Sends `Frame.tap`. */
  async _tap(
    apiName: ApiName,
    selector: string,
    options: TapOptions,
  ): Promise<void> {
    await this._connection.send(
      apiName,
      this._guid,
      'tap',
      pickParams(FRAME_TAP_KEYS, {
        selector,
        ...options,
      }) as FrameTapParams,
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

/** Playwright's waiter timeout (`client/frame.ts` `_setupNavigationWaiter`): a `TimeoutError`
 * naming the whole timeout, with the API call's prefix. */
function navigationTimeout(apiName: ApiName, timeout: number): TimeoutError {
  return new TimeoutError(`${apiName}: Timeout ${timeout}ms exceeded.`)
}

/** Playwright's `_wrapApiCall`, which prefixes every error thrown inside an API call. */
function prefixed(apiName: ApiName, error: unknown): unknown {
  if (error instanceof Error) error.message = `${apiName}: ${error.message}`
  return error
}

/**
 * A predicate's verdict, truthy or not, or the waiter's timeout once `deadline` (0 for none)
 * passes while it is still pending, as Playwright's waiter rejects a wait whose predicate has not
 * answered by its deadline.
 */
async function verdictBefore(
  verdict: boolean | Promise<boolean>,
  deadline: number,
  timedOut: () => TimeoutError,
): Promise<boolean> {
  const settled = Promise.resolve(verdict).then((value) => !!value)
  if (!deadline) return await settled
  const ms = deadline - Date.now()
  if (ms <= 0) throw timedOut()
  let timer: ReturnType<typeof setTimeout> | undefined
  try {
    return await Promise.race([
      settled,
      new Promise<never>((_, reject) => {
        timer = setTimeout(() => reject(timedOut()), ms)
      }),
    ])
  } finally {
    clearTimeout(timer)
  }
}
