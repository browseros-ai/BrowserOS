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
 * - Text collections run as `$$eval` in the page's main world.
 */

/** Playwright's `WaitForFunctionOptions`: `polling` is `'raf'` (default) or an interval in ms. */
export type WaitForFunctionOptions = {
  polling?: number | 'raf'
  timeout?: number
  signal?: SignalLike | null
}

import type {
  FrameAriaSnapshotParams,
  FrameAriaSnapshotResult,
  FrameBlurParams,
  FrameCheckParams,
  FrameClickParams,
  FrameDblclickParams,
  FrameDispatchEventParams,
  FrameDragAndDropParams,
  FrameFillParams,
  FrameFocusParams,
  FrameGetAttributeParams,
  FrameGetAttributeResult,
  FrameGotoParams,
  FrameHideHighlightParams,
  FrameHighlightParams,
  FrameHoverParams,
  FrameInitializer,
  FrameInnerHTMLParams,
  FrameInnerHTMLResult,
  FrameInnerTextParams,
  FrameInnerTextResult,
  FrameInputValueParams,
  FrameIsCheckedParams,
  FrameIsDisabledParams,
  FrameIsEditableParams,
  FrameIsEnabledParams,
  FrameIsHiddenParams,
  FrameIsVisibleParams,
  FramePressParams,
  FrameQueryCountParams,
  FrameSelectOptionParams,
  FrameTapParams,
  FrameTextContentParams,
  FrameTextContentResult,
  FrameTitleResult,
  FrameTypeParams,
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
import { TimeoutError } from './errors'
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
import {
  NEO_SELECT_TEXT_KEYS,
  type NeoNavigationResult,
  type NeoNextNavigationResult,
  type NeoRect,
  type NeoSelectTextParams,
  serializeUrlMatch,
} from './neo-commands'
import { type Response, responseFor } from './network'
import type { Page } from './page'
import { serializePlainArgument } from './plain-argument'
import {
  FRAME_ARIA_SNAPSHOT_KEYS,
  FRAME_BLUR_KEYS,
  FRAME_CHECK_KEYS,
  FRAME_CLICK_KEYS,
  FRAME_DBLCLICK_KEYS,
  FRAME_DISPATCH_EVENT_KEYS,
  FRAME_DRAG_AND_DROP_KEYS,
  FRAME_FILL_KEYS,
  FRAME_FOCUS_KEYS,
  FRAME_GET_ATTRIBUTE_KEYS,
  FRAME_GOTO_KEYS,
  FRAME_HIDE_HIGHLIGHT_KEYS,
  FRAME_HIGHLIGHT_KEYS,
  FRAME_HOVER_KEYS,
  FRAME_INNER_HTML_KEYS,
  FRAME_INNER_TEXT_KEYS,
  FRAME_INPUT_VALUE_KEYS,
  FRAME_IS_CHECKED_KEYS,
  FRAME_IS_DISABLED_KEYS,
  FRAME_IS_EDITABLE_KEYS,
  FRAME_IS_ENABLED_KEYS,
  FRAME_IS_HIDDEN_KEYS,
  FRAME_IS_VISIBLE_KEYS,
  FRAME_PRESS_KEYS,
  FRAME_QUERY_COUNT_KEYS,
  FRAME_SELECT_OPTION_KEYS,
  FRAME_TAP_KEYS,
  FRAME_TEXT_CONTENT_KEYS,
  FRAME_TYPE_KEYS,
  FRAME_UNCHECK_KEYS,
  FRAME_WAIT_FOR_SELECTOR_KEYS,
  pickParams,
} from './protocol'
import { get } from './runtime-bridge'
import {
  convertSelectOptionValues,
  type SelectOptionValues,
} from './select-option'
import type {
  SignalLike,
  TimeoutInput,
  TimeoutOptions,
} from './timeout-settings'
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

/** `innerHTML`'s options as a script passes them. */
export type InnerHTMLOptions = Omit<FrameInnerHTMLParams, 'selector'> & {
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
export type WaitForURLOptions = { waitUntil?: LifecycleEvent } & TimeoutInput

/** Playwright's `WaitForNavigationOptions` (`client/frame.ts`). */
export type WaitForNavigationOptions = {
  url?: NavigationURLMatch
  waitUntil?: LifecycleEvent
} & TimeoutInput

/** `waitForLoadState`'s options, as Playwright's `TimeoutOptions`. */
export type WaitForLoadStateOptions = TimeoutInput

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
  /**
   * This option has no effect.
   * @deprecated This option has no effect.
   */
  noWaitAfter?: boolean
}
export type PressOptions = Omit<FramePressParams, 'selector' | 'key'> & {
  timeout?: number
}
/** `Locator.dragTo`'s and `Page/Frame.dragAndDrop`'s options. */
export type DragAndDropOptions = Omit<
  FrameDragAndDropParams,
  'source' | 'target'
> & {
  timeout?: number
}
/** `Locator.pressSequentially`'s and the deprecated `type`s' options. */
export type TypeOptions = Omit<FrameTypeParams, 'selector' | 'text'> & {
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
/** Playwright's `isHidden` takes `timeout` and ignores it, as `isVisible` does. */
export type IsHiddenOptions = Omit<FrameIsHiddenParams, 'selector'> & {
  timeout?: number
}
export type IsEnabledOptions = Omit<FrameIsEnabledParams, 'selector'> & {
  timeout?: number
}
export type IsDisabledOptions = Omit<FrameIsDisabledParams, 'selector'> & {
  timeout?: number
}
export type IsEditableOptions = Omit<FrameIsEditableParams, 'selector'> & {
  timeout?: number
}
export type CheckOptions = Omit<FrameCheckParams, 'selector'> & {
  timeout?: number
  /**
   * This option has no effect.
   * @deprecated This option has no effect.
   */
  noWaitAfter?: boolean
}
export type UncheckOptions = Omit<FrameUncheckParams, 'selector'> & {
  timeout?: number
  /**
   * This option has no effect.
   * @deprecated This option has no effect.
   */
  noWaitAfter?: boolean
}
export type SelectOptionOptions = Omit<
  FrameSelectOptionParams,
  'selector' | 'elements' | 'options'
> & {
  timeout?: number
  /**
   * This option has no effect.
   * @deprecated This option has no effect.
   */
  noWaitAfter?: boolean
}

/** `focus`'s and `blur`'s options as a script passes them. */
export type FocusOptions = Omit<FrameFocusParams, 'selector'> & {
  timeout?: number
}

/** `dispatchEvent`'s options as a script passes them. */
export type DispatchEventOptions = Omit<
  FrameDispatchEventParams,
  'selector' | 'type' | 'eventInit'
> & { timeout?: number }

/** `selectText`'s options, as `ElementHandle.selectText` takes them. */
export type SelectTextOptions = { force?: boolean; timeout?: number }

/** The options of the `Locator` methods Playwright serves through an `ElementHandle`. */
export type ElementTimeoutOptions = { timeout?: number }

/** What a frame event says of its frame (`Frame._during`). */
export interface FrameEventState {
  url: string
  name: string
  detached: boolean
}

export class Frame {
  /** The opaque guid neo-runtime routes this frame's browser calls by. */
  readonly _guid: string
  readonly _page: Page
  readonly _connection: Connection
  #lastUrl: string
  #lastName: string
  /** While the listeners of an event about this frame run, what the event says of it. */
  #during: FrameEventState | undefined

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

  /**
   * Runs `emit`, the listeners of an event about this frame, with the frame answering `url()`,
   * `name()` and `isDetached()` as the event describes it: what Playwright's client frame
   * answers then, its state updated event by event. A listener's reads after an `await` are live.
   */
  _during(state: FrameEventState, emit: () => void): void {
    const outer = this.#during
    this.#during = state
    try {
      emit()
    } finally {
      this.#during = outer
    }
  }

  url(): string {
    if (this.#during) return this.#during.url
    const url = get(this._guid, 'url')
    if (url !== null) this.#lastUrl = url
    return this.#lastUrl
  }

  name(): string {
    if (this.#during) return this.#during.name
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
    if (this.#during) return this.#during.detached
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
      this._page._timeoutSettings.navigationTimeout(options, apiName),
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
        this._page._timeoutSettings.navigationTimeout(waitOptions, apiName),
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
      this._page._timeoutSettings.navigationTimeout(waitOptions, apiName),
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
    const timeoutOptions = this._page._timeoutSettings.navigationTimeout(
      options,
      apiName,
    )
    const result =
      typeof options.url === 'function'
        ? await this._navigationMatching(
            apiName,
            options.url,
            waitUntil,
            timeoutOptions,
            true,
          )
        : await this._connection.sendNeo(
            apiName,
            this._guid,
            'neo:waitForNavigation',
            { url: serializeUrlMatch(apiName, options.url), waitUntil },
            timeoutOptions,
          )
    const guid = result?.response?.guid
    return guid ? responseFor(this._connection, guid, this) : null
  }

  /**
   * A URL predicate's wait (`neo-commands.ts`, DECIDED 3), as Playwright's waiter runs it: each
   * navigation's predicate is tested on its own, sync or async, and the first to accept wins,
   * whatever earlier answers are still pending (a URL the parser rejects is never accepted). One
   * `neo:nextNavigation` is always waiting, so later navigations keep arriving and the page's
   * closure or a failed navigation rejects the wait even with no timeout. `respond` asks for the
   * winning navigation's `Response`. `timeout` (0 for none) bounds the whole wait: each call gets
   * what is left, and the deadline also rejects while only predicates are pending. `signal` goes
   * with each `next` and the finish, so its abort rejects the waiting one. However the wait
   * ends, its cursor ends with it: by the finish, or by a release when the realm ended it. Rust
   * then answers the next still waiting, and the wait settles only once that answer is in, so a
   * settled wait holds none of the run's call capacity (neo-runtime `dispatch.rs`
   * `MAX_CALLS_IN_FLIGHT`) or live-wait capacity (`history.rs` `MAX_CURSORS`).
   */
  async _navigationMatching(
    apiName: ApiName,
    predicate: URLPredicate,
    waitUntil: LifecycleEvent,
    { timeout, signal }: TimeoutOptions,
    respond: boolean,
  ): Promise<NeoNavigationResult> {
    const deadline = timeout ? Date.now() + timeout : 0
    const left = () => {
      if (!deadline) return 0
      const ms = deadline - Date.now()
      if (ms <= 0) throw navigationTimeout(apiName, timeout)
      return ms
    }
    let cursor: number | undefined
    /** Set once a finish Rust answered ended the cursor. */
    let finished = false
    const finish = async (id: number, index: number) => {
      const result = await this.#finishNavigation(
        apiName,
        id,
        index,
        waitUntil,
        { timeout: left(), signal },
      )
      finished = true
      return result
    }
    const next = () => {
      const call = this._connection.sendNeo(
        apiName,
        this._guid,
        'neo:nextNavigation',
        cursor === undefined
          ? { response: respond }
          : { cursor, response: respond },
        { timeout: left(), signal },
      )
      call.catch(() => {})
      return call
    }
    let timer: ReturnType<typeof setTimeout> | undefined
    const expired = new Promise<never>((_, reject) => {
      if (deadline)
        timer = setTimeout(
          () => reject(navigationTimeout(apiName, timeout)),
          Math.max(0, deadline - Date.now()),
        )
    })
    expired.catch(() => {})
    /** Each pending predicate's answer, by the index of the navigation it tests. */
    const verdicts = new Map<number, Promise<Verdict>>()
    let waiting = next()
    try {
      for (;;) {
        const step: Verdict | { reply: NeoNextNavigationResult } =
          await Promise.race([
            waiting.then((reply) => ({ reply })),
            ...verdicts.values(),
            expired,
          ])
        if ('reply' in step) {
          const { cursor: id, index, url } = step.reply
          cursor = id
          if (index === undefined || url === undefined)
            throw new Error(
              'neo-sdk: neo-runtime ended this navigation wait under it',
            )
          waiting = next()
          const parsed = urlFor(url)
          if (!parsed) continue
          let verdict: boolean | Promise<boolean>
          try {
            verdict = predicate(parsed)
          } catch (error) {
            throw prefixed(apiName, error)
          }
          if (isPromiseLike(verdict)) {
            const answer = Promise.resolve(verdict).then(
              (value) => ({ index, accepted: !!value }),
              (error) => {
                throw prefixed(apiName, error)
              },
            )
            answer.catch(() => {})
            verdicts.set(index, answer)
            continue
          }
          if (!verdict) continue
          return await finish(id, index)
        }
        verdicts.delete(step.index)
        if (!step.accepted || cursor === undefined) continue
        return await finish(cursor, step.index)
      }
    } catch (error) {
      // Each step ran under what was left of `timeout`; its message names the whole timeout.
      if (error instanceof TimeoutError && deadline)
        error.message = error.message.replace(
          /Timeout [\d.]+ms exceeded\./,
          `Timeout ${timeout}ms exceeded.`,
        )
      throw error
    } finally {
      clearTimeout(timer)
      await this.#endWait(apiName, cursor, finished, waiting)
    }
  }

  /**
   * Ends a predicate wait's cursor however the wait ended, unless its finish did: a release, which
   * Rust ignores for a cursor it already ended. Then waits for Rust's answer to the next still
   * waiting, which comes once the cursor ended; a first next it never answered shares the wait's
   * own deadline. Only a release that failed (the page closed) leaves that answer unknown.
   */
  async #endWait(
    apiName: ApiName,
    cursor: number | undefined,
    finished: boolean,
    waiting: Promise<NeoNextNavigationResult>,
  ): Promise<void> {
    let ended = finished
    if (cursor !== undefined && !ended)
      ended = await this._connection
        .sendNeo(apiName, this._guid, 'neo:releaseNavigation', { cursor }, 0)
        .then(
          () => true,
          () => false,
        )
    if (ended || cursor === undefined) await waiting.catch(() => {})
  }

  /** Sends `neo:finishNavigation` for the navigation the predicate accepted. */
  async #finishNavigation(
    apiName: ApiName,
    cursor: number,
    index: number,
    waitUntil: LifecycleEvent,
    timeout: TimeoutOptions,
  ): Promise<NeoNavigationResult> {
    return await this._connection.sendNeo(
      apiName,
      this._guid,
      'neo:finishNavigation',
      { cursor, index, waitUntil },
      timeout,
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
      this._page._timeoutSettings.navigationTimeout(options ?? {}, apiName),
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
      this._page._timeoutSettings.timeout(options, apiName),
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
    options: TimeoutInput,
  ): Promise<R> {
    const result = (await this._connection.send(
      apiName,
      this._guid,
      'waitForSelector',
      { selector, strict: true, state: 'attached' },
      this._page._timeoutSettings.timeout(options, apiName),
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
      this._page._timeoutSettings.timeout(options ?? {}, apiName),
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
      this._page._timeoutSettings.timeout(options ?? {}, apiName),
    )) as FrameInnerTextResult
    return result.value
  }

  /** Not strict, as Playwright's `frame.innerHTML`; waits for the first match. */
  async innerHTML(
    selector: string,
    options: InnerHTMLOptions = {},
  ): Promise<string> {
    return await this._innerHTML('frame.innerHTML', selector, options)
  }

  /** Sends `Frame.innerHTML` under the action timeout. */
  async _innerHTML(
    apiName: ApiName,
    selector: string,
    options: InnerHTMLOptions,
  ): Promise<string> {
    const result = (await this._connection.send(
      apiName,
      this._guid,
      'innerHTML',
      pickParams(FRAME_INNER_HTML_KEYS, {
        selector,
        ...options,
      }) as FrameInnerHTMLParams,
      this._page._timeoutSettings.timeout(options ?? {}, apiName),
    )) as FrameInnerHTMLResult
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
      this._page._timeoutSettings.timeout(options ?? {}, apiName),
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

  /** Playwright's allInnerTexts is $$eval in the main world; undefined entries remain values. */
  async _allInnerTexts(apiName: ApiName, selector: string): Promise<string[]> {
    const result = await this._connection.sendEval(
      apiName,
      this._guid,
      'evalOnSelectorAll',
      {
        selector,
        expression: 'ee => ee.map(e => e.innerText)',
        isFunction: true,
        arg: serializeArgument(undefined),
      },
      0,
    )
    return parseResult(result?.value) as string[]
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
    options: { strict?: boolean } | undefined,
  ): Promise<ElementHandle | null> {
    // Playwright sends `$` with `kNoTimeout`, so it reads no `signal`, and neither does Neo.
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
      this._page._timeoutSettings.timeout(options, apiName),
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
    options: WaitForSelectorOptions & {
      strict?: boolean
      signal?: SignalLike | null
    },
  ): Promise<ElementHandle | null> {
    const result = (await this._connection.send(
      apiName,
      this._guid,
      'waitForSelector',
      pickParams(FRAME_WAIT_FOR_SELECTOR_KEYS, {
        selector,
        ...options,
      }) as FrameWaitForSelectorParams,
      this._page._timeoutSettings.timeout(options, apiName),
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
      this._page._timeoutSettings.timeout(options, apiName),
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
      this._page._timeoutSettings.timeout(options ?? {}, apiName),
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
      this._page._timeoutSettings.timeout(options ?? {}, apiName),
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
      this._page._timeoutSettings.timeout(options ?? {}, apiName),
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
      this._page._timeoutSettings.timeout(options ?? {}, apiName),
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
      this._page._timeoutSettings.timeout(options ?? {}, apiName),
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
      this._page._timeoutSettings.timeout(options ?? {}, apiName),
    )
  }

  /** A move and press on `source`, then a move and release on `target`, in one timeout. */
  async dragAndDrop(
    source: string,
    target: string,
    options: DragAndDropOptions = {},
  ): Promise<void> {
    return await this._dragAndDrop('frame.dragAndDrop', source, target, options)
  }

  /** Synchronously reaches `send` so it captures the script's call site. */
  async _dragAndDrop(
    apiName: ApiName,
    source: string,
    target: string,
    options: DragAndDropOptions,
  ): Promise<void> {
    await this._connection.send(
      apiName,
      this._guid,
      'dragAndDrop',
      pickParams(FRAME_DRAG_AND_DROP_KEYS, {
        source,
        target,
        ...options,
      }) as FrameDragAndDropParams,
      this._page._timeoutSettings.timeout(options ?? {}, apiName),
    )
  }

  /** Deprecated in Playwright for `Locator.pressSequentially`, and served the same way. */
  async type(
    selector: string,
    text: string,
    options: TypeOptions = {},
  ): Promise<void> {
    return await this._type('frame.type', selector, text, options)
  }

  /** Synchronously reaches `send` so it captures the script's call site. */
  async _type(
    apiName: ApiName,
    selector: string,
    text: string,
    options: TypeOptions,
  ): Promise<void> {
    await this._connection.send(
      apiName,
      this._guid,
      'type',
      pickParams(FRAME_TYPE_KEYS, {
        selector,
        text,
        ...options,
      }) as FrameTypeParams,
      this._page._timeoutSettings.timeout(options ?? {}, apiName),
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
      this._page._timeoutSettings.timeout(options ?? {}, apiName),
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

  /** Playwright's `kNoTimeout`: visibility reads once and ignores `options.timeout` and
   * `options.signal`. */
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

  async isHidden(
    selector: string,
    options: IsHiddenOptions = {},
  ): Promise<boolean> {
    return await this._isHidden('frame.isHidden', selector, options)
  }

  /** Playwright's `kNoTimeout`: `isHidden` reads once, as `isVisible`, and ignores
   * `options.timeout` and `options.signal`. */
  async _isHidden(
    apiName: ApiName,
    selector: string,
    options: IsHiddenOptions,
  ): Promise<boolean> {
    const result = await this._connection.send(
      apiName,
      this._guid,
      'isHidden',
      pickParams(FRAME_IS_HIDDEN_KEYS, {
        selector,
        ...options,
      }) as FrameIsHiddenParams,
      0,
    )
    if (result === undefined)
      throw new Error('neo-sdk: Frame.isHidden contract error: missing result')
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
      this._page._timeoutSettings.timeout(options ?? {}, apiName),
    )
    if (result === undefined)
      throw new Error('neo-sdk: Frame.isChecked contract error: missing result')
    return result.value
  }

  async isEnabled(
    selector: string,
    options: IsEnabledOptions = {},
  ): Promise<boolean> {
    return await this._isEnabled('frame.isEnabled', selector, options)
  }

  /** Waits for the first match under the caller's action timeout, as `isChecked`. */
  async _isEnabled(
    apiName: ApiName,
    selector: string,
    options: IsEnabledOptions,
  ): Promise<boolean> {
    const result = await this._connection.send(
      apiName,
      this._guid,
      'isEnabled',
      pickParams(FRAME_IS_ENABLED_KEYS, {
        selector,
        ...options,
      }) as FrameIsEnabledParams,
      this._page._timeoutSettings.timeout(options ?? {}, apiName),
    )
    if (result === undefined)
      throw new Error('neo-sdk: Frame.isEnabled contract error: missing result')
    return result.value
  }

  async isDisabled(
    selector: string,
    options: IsDisabledOptions = {},
  ): Promise<boolean> {
    return await this._isDisabled('frame.isDisabled', selector, options)
  }

  /** Waits for the first match under the caller's action timeout, as `isChecked`. */
  async _isDisabled(
    apiName: ApiName,
    selector: string,
    options: IsDisabledOptions,
  ): Promise<boolean> {
    const result = await this._connection.send(
      apiName,
      this._guid,
      'isDisabled',
      pickParams(FRAME_IS_DISABLED_KEYS, {
        selector,
        ...options,
      }) as FrameIsDisabledParams,
      this._page._timeoutSettings.timeout(options ?? {}, apiName),
    )
    if (result === undefined)
      throw new Error(
        'neo-sdk: Frame.isDisabled contract error: missing result',
      )
    return result.value
  }

  async isEditable(
    selector: string,
    options: IsEditableOptions = {},
  ): Promise<boolean> {
    return await this._isEditable('frame.isEditable', selector, options)
  }

  /** Waits for the first match under the caller's action timeout; an element that cannot be
   * editable rejects at once, as Playwright's page code throws. */
  async _isEditable(
    apiName: ApiName,
    selector: string,
    options: IsEditableOptions,
  ): Promise<boolean> {
    const result = await this._connection.send(
      apiName,
      this._guid,
      'isEditable',
      pickParams(FRAME_IS_EDITABLE_KEYS, {
        selector,
        ...options,
      }) as FrameIsEditableParams,
      this._page._timeoutSettings.timeout(options ?? {}, apiName),
    )
    if (result === undefined)
      throw new Error(
        'neo-sdk: Frame.isEditable contract error: missing result',
      )
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
      this._page._timeoutSettings.timeout(options ?? {}, apiName),
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
      this._page._timeoutSettings.timeout(options ?? {}, apiName),
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
      this._page._timeoutSettings.timeout(options ?? {}, apiName),
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

  /** Not strict, as Playwright's `frame.focus`; waits for the element, then focuses it. */
  async focus(selector: string, options: FocusOptions = {}): Promise<void> {
    return await this._focus('frame.focus', selector, options)
  }

  /** Sends `Frame.focus` under the action timeout. */
  async _focus(
    apiName: ApiName,
    selector: string,
    options: FocusOptions,
  ): Promise<void> {
    await this._connection.send(
      apiName,
      this._guid,
      'focus',
      pickParams(FRAME_FOCUS_KEYS, {
        selector,
        ...options,
      }) as FrameFocusParams,
      this._page._timeoutSettings.timeout(options ?? {}, apiName),
    )
  }

  /** Sends `Frame.blur` under the action timeout; behind `locator.blur` only, as Playwright's. */
  async _blur(
    apiName: ApiName,
    selector: string,
    options: FocusOptions,
  ): Promise<void> {
    await this._connection.send(
      apiName,
      this._guid,
      'blur',
      pickParams(FRAME_BLUR_KEYS, {
        selector,
        ...options,
      }) as FrameBlurParams,
      this._page._timeoutSettings.timeout(options ?? {}, apiName),
    )
  }

  /** Not strict, as Playwright's `frame.dispatchEvent`; waits for the element. */
  async dispatchEvent(
    selector: string,
    type: string,
    eventInit?: unknown,
    options: DispatchEventOptions = {},
  ): Promise<void> {
    return await this._dispatchEvent(
      'frame.dispatchEvent',
      selector,
      type,
      eventInit,
      options,
    )
  }

  /** Sends `Frame.dispatchEvent` under the action timeout, `eventInit` serialized first, so a
   * value Neo cannot send throws before anything is sent. */
  async _dispatchEvent(
    apiName: ApiName,
    selector: string,
    type: string,
    eventInit: unknown,
    options: DispatchEventOptions,
  ): Promise<void> {
    await this._connection.send(
      apiName,
      this._guid,
      'dispatchEvent',
      pickParams(FRAME_DISPATCH_EVENT_KEYS, {
        selector,
        type,
        eventInit: serializePlainArgument(eventInit),
        ...options,
      }) as FrameDispatchEventParams,
      this._page._timeoutSettings.timeout(options ?? {}, apiName),
    )
  }

  /** Sends `neo:scrollIntoViewIfNeeded` strictly, under the action timeout. */
  async _scrollIntoViewIfNeeded(
    apiName: ApiName,
    selector: string,
    options: ElementTimeoutOptions,
  ): Promise<void> {
    await this._connection.sendNeo(
      apiName,
      this._guid,
      'neo:scrollIntoViewIfNeeded',
      { selector, strict: true },
      this._page._timeoutSettings.timeout(options ?? {}, apiName),
    )
  }

  /** Sends `neo:selectText` strictly, under the action timeout. */
  async _selectText(
    apiName: ApiName,
    selector: string,
    options: SelectTextOptions,
  ): Promise<void> {
    const opts = options ?? {}
    await this._connection.sendNeo(
      apiName,
      this._guid,
      'neo:selectText',
      pickParams(NEO_SELECT_TEXT_KEYS, {
        selector,
        strict: true,
        force: opts.force,
      }) as NeoSelectTextParams,
      this._page._timeoutSettings.timeout(opts, apiName),
    )
  }

  /** Sends `neo:boundingBox` strictly, under the action timeout: the border box, or `null`. */
  async _boundingBox(
    apiName: ApiName,
    selector: string,
    options: ElementTimeoutOptions,
  ): Promise<NeoRect | null> {
    const result = await this._connection.sendNeo(
      apiName,
      this._guid,
      'neo:boundingBox',
      { selector, strict: true },
      this._page._timeoutSettings.timeout(options ?? {}, apiName),
    )
    const box = result?.value
    // Playwright's `Rect` key order (`CRPage.getBoundingBox`), whatever order the wire used.
    return box
      ? { x: box.x, y: box.y, width: box.width, height: box.height }
      : null
  }

  /** Sends `Frame.highlight` with no timeout, as Playwright's `_highlight` (`kNoTimeout`). */
  async _highlight(
    apiName: ApiName,
    selector: string,
    style?: string,
  ): Promise<void> {
    await this._connection.send(
      apiName,
      this._guid,
      'highlight',
      pickParams(FRAME_HIGHLIGHT_KEYS, {
        selector,
        style,
      }) as FrameHighlightParams,
      0,
    )
  }

  /** Sends `Frame.hideHighlight` with no timeout, as Playwright's `_hideHighlight`. */
  async _hideHighlight(apiName: ApiName, selector: string): Promise<void> {
    await this._connection.send(
      apiName,
      this._guid,
      'hideHighlight',
      pickParams(FRAME_HIDE_HIGHLIGHT_KEYS, {
        selector,
      }) as FrameHideHighlightParams,
      0,
    )
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

/** One predicate's answer for the navigation at `index` of its wait. */
type Verdict = { index: number; accepted: boolean }

/** Whether a predicate answered asynchronously: Playwright's waiter awaits any thenable. */
function isPromiseLike(value: unknown): value is PromiseLike<boolean> {
  return (
    (typeof value === 'object' || typeof value === 'function') &&
    value !== null &&
    typeof (value as { then?: unknown }).then === 'function'
  )
}
