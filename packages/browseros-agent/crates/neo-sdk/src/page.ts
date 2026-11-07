import type {
  LifecycleEvent,
  PageCloseParams,
  PageConsoleMessagesParams,
  PageGoBackParams,
  PageGoForwardParams,
  PagePageErrorsParams,
  PageRegisterLocatorHandlerParams,
  PageRegisterLocatorHandlerResult,
  PageReloadParams,
  PageResolveLocatorHandlerNoReplyParams,
  PageUnregisterLocatorHandlerParams,
  ResponseChannel,
} from '@browseros/neo-runtime-spec/serve'
import type { ByRoleOptions } from '../vendor/playwright/isomorphic/locatorUtils'
import { isString } from '../vendor/playwright/isomorphic/stringUtils'
import type { BrowserContext } from './browser-context'
import type { Connection } from './connection'
import { ConsoleMessage } from './console-message'
import type { ElementHandle } from './element-handle'
import {
  isTargetClosedError,
  parseSerializedError,
  targetClosedError,
} from './errors'
import { ChannelOwner, type EventTable } from './events'
import {
  type AriaSnapshotOptions,
  type CheckOptions,
  type ClickOptions,
  type DblclickOptions,
  type FillOptions,
  Frame,
  type GetAttributeOptions,
  type GotoOptions,
  type HoverOptions,
  type InnerHTMLOptions,
  type InnerTextOptions,
  type InputValueOptions,
  type IsCheckedOptions,
  type IsDisabledOptions,
  type IsEditableOptions,
  type IsEnabledOptions,
  type IsHiddenOptions,
  type IsVisibleOptions,
  type NavigationURLMatch,
  type PressOptions,
  type SelectOptionOptions,
  type TapOptions,
  type TextContentOptions,
  type UncheckOptions,
  type URLMatch,
  verifyLoadState,
  type WaitForFunctionOptions,
  type WaitForLoadStateOptions,
  type WaitForNavigationOptions,
  type WaitForSelectorOptions,
  type WaitForURLOptions,
} from './frame'
import type { FrameLocator } from './frame-locator'
import {
  assertMaxArguments,
  type EvaluateOptions,
  type JSHandle,
  type PageElement,
  type PageFunction,
  type PageFunctionOn,
} from './js-handle'
import type { Locator, LocatorOptions } from './locator'
import { serializeUrlMatch } from './neo-commands'
import { type Response, responseFor } from './network'
import {
  PAGE_CLOSE_KEYS,
  PAGE_CONSOLE_MESSAGES_KEYS,
  PAGE_GO_BACK_KEYS,
  PAGE_GO_FORWARD_KEYS,
  PAGE_PAGE_ERRORS_KEYS,
  PAGE_REGISTER_LOCATOR_HANDLER_KEYS,
  PAGE_RELOAD_KEYS,
  PAGE_RESOLVE_LOCATOR_HANDLER_KEYS,
  PAGE_UNREGISTER_LOCATOR_HANDLER_KEYS,
  pickParams,
} from './protocol'
import { type CreatedFrame, get, matchesURL } from './runtime-bridge'
import type { SelectOptionValues } from './select-option'
import { TimeoutSettings } from './timeout-settings'
import { Waiter, type WaitForEventOptions, wrapApiCall } from './waiter'

/**
 * Neo: the page's listener events, by Playwright's names (`Events.Page`): what Neo produces,
 * and Playwright's others, which other slices of this build add.
 */
const PAGE_EVENTS: EventTable = {
  receiver: 'Page',
  served: [
    'console',
    'pageerror',
    'dialog',
    'dialogclosed',
    'close',
    'crash',
    'load',
    'domcontentloaded',
    'frameattached',
    'framedetached',
    'framenavigated',
    'popup',
  ],
  quiet: [],
  unserved: [
    'download',
    'filechooser',
    'request',
    'response',
    'requestfailed',
    'requestfinished',
    'websocket',
    'worker',
  ],
}

/** A frame event's params from Rust (`frame_events.rs`): the frame's record, and for a
 * navigation the URL and name it committed, as Playwright's `navigated` event carries them. */
interface FrameEventParams {
  frame: CreatedFrame
  url?: string
  name?: string
}

/** `consoleMessages` and `pageErrors`' options: Playwright's `ConsoleMessagesFilter`. */
export interface ConsoleMessagesOptions {
  filter?: 'all' | 'since-navigation'
}

/** `reload`, `goBack` and `goForward`'s options as a script passes them: the wire params plus
 * `timeout`, as Playwright's `channels.PageReloadOptions & TimeoutOptions`. */
export type HistoryOptions = { waitUntil?: LifecycleEvent; timeout?: number }

/**
 * Playwright's `Page`, backed by live reads from its own tab, which its guid names. Its
 * selector builders, `goto`, `title`, `textContent`, `innerText`, `innerHTML`, `getAttribute`,
 * state reads, browser actions, `waitForURL`, `waitForLoadState` and `waitForNavigation` forward
 * to the main frame, as Playwright's do (`client/page.ts`); `ariaSnapshot` snapshots the main
 * frame's document; `reload`, `goBack` and `goForward` are the page's own commands. Only the run's
 * `BrowserContext` builds one, once per guid.
 *
 * Known limitation: once the tab is closed, `url()` returns the last URL this object returned,
 * not the last URL the tab committed. Navigate `/a` to `/final`, close the tab before the
 * script calls `url()` again, and `url()` returns `/a`; Playwright returns `/final`. No CDP
 * source answers after a close, so matching Playwright here needs event tracking, which the
 * no-cache design leaves out (sync-getters-design.md, "Known limitation").
 *
 * Deviation: `client/page.ts` `Page.constructor`: `this.once(Events.Page.Close, () => this._closedOrCrashedScope.close(this._closeErrorWithReason()));`
 * Playwright's page holds a `close` and a `crash` listener of its own from its first moment,
 * which `eventNames()` and `listenerCount` report; Neo's holds none, so a page nobody listens to
 * costs the run nothing.
 */
export class Page extends ChannelOwner {
  readonly #guid: string
  /**
   * The only tab state a `Page` keeps. Playwright still answers `url()` after the page closes,
   * with the last URL, while every CDP source errors; seeded with the tab's URL in the record
   * the run was handed (`RunInit.url` for the run page).
   */
  #lastUrl: string
  readonly #context: BrowserContext
  readonly #connection: Connection
  /** Built once: `Locator` throws `… must belong to the same frame.` when two locators hold
   * different `Frame` objects, so `mainFrame()` must always return this one. */
  readonly #mainFrame: Frame
  /** Stable identity, including detached frames a script may still hold. */
  readonly #frames = new Map<string, Frame>()
  readonly _timeoutSettings: TimeoutSettings
  /** Set when this page's `close` ran (`_onClose`): what Playwright's client flips then. */
  #closedLocally = false
  /** Playwright's `_closeReason`: what `close({ reason })` was given. */
  #closeReason: string | undefined

  /** `guid` and `mainFrameGuid` come from one record Rust minted (`page@T`, `frame@T`), so a
   * page's calls and its frame's calls reach the same tab. */
  constructor(
    guid: string,
    mainFrameGuid: string,
    url: string,
    context: BrowserContext,
    connection: Connection,
  ) {
    super(guid, context._pipe, PAGE_EVENTS)
    this.#guid = guid
    this.#lastUrl = url
    this.#context = context
    this.#connection = connection
    this._timeoutSettings = new TimeoutSettings(context._timeoutSettings)
    this.#mainFrame = new Frame(mainFrameGuid, this, connection, {
      url,
      name: '',
    })
    this.#frames.set(mainFrameGuid, this.#mainFrame)
  }

  url(): string {
    if (this.#closedLocally) return this.#lastUrl
    const url = get(this.#guid, 'url')
    if (url !== null) this.#lastUrl = url
    return this.#lastUrl
  }

  /**
   * Makes this the active tab of its window.
   *
   * Deviation: `server/chromium/crPage.ts` `CRPage.bringToFront`: `await this._mainFrameSession._client.send('Page.bringToFront');`
   * Playwright sends `Page.bringToFront` alone. Neo also activates the tab in BrowserOS's tab
   * strip when it is not the active one, which `Page.bringToFront` leaves as it was.
   */
  async bringToFront(): Promise<void> {
    await this.#connection.sendPage(
      'page.bringToFront',
      this.#guid,
      'bringToFront',
      {},
      0,
    )
  }

  /**
   * Playwright's client `close`. By default it closes the tab and resolves once the tab is
   * gone, so `isClosed()` is then true; closing a closed page resolves. `runBeforeUnload` asks
   * the page to close itself instead, which runs its `beforeunload` handlers and resolves
   * without waiting for the close; on a closed page it throws. Both calls have no timeout.
   *
   * Playwright also records `reason` and that `close` was called, for its event waiters and
   * route handlers. Neo has neither: the reason goes to Rust.
   */
  async close(
    options: { runBeforeUnload?: boolean; reason?: string } = {},
  ): Promise<void> {
    this.#closeReason = options.reason
    try {
      if (options.runBeforeUnload)
        await this.#connection.sendPage(
          'page.close',
          this.#guid,
          'runBeforeUnload',
          {},
          0,
        )
      else
        await this.#connection.sendPage(
          'page.close',
          this.#guid,
          'close',
          pickParams(PAGE_CLOSE_KEYS, {
            reason: options.reason,
          }) as PageCloseParams,
          0,
        )
    } catch (error) {
      if (isTargetClosedError(error) && !options.runBeforeUnload) return
      throw error
    }
  }

  isClosed(): boolean {
    return this.#closedLocally || get(this.#guid, 'isClosed')
  }

  /** Whether this page's `close` ran, from its tab's close or its context's. */
  _isClosedLocally(): boolean {
    return this.#closedLocally
  }

  /** Playwright's `_closeErrorWithReason` reason: the page's own, else its context's. */
  _closeReason(): string | undefined {
    return this.#closeReason ?? this.#context._closeReason()
  }

  /** Playwright's page channel handlers (`client/page.ts` constructor), and its main frame's
   * `loadstate` and every frame's `navigated` (`client/frame.ts` constructor), which Neo sends
   * on the page. */
  _onEvent(method: string, params: unknown): void {
    if (method === 'close') this._onClose()
    else if (method === 'crash') this.emit('crash', this)
    else if (method === 'load') {
      this.emit('load', this)
      this.#context.emit('pageload', this)
    } else if (method === 'domcontentloaded')
      this.emit('domcontentloaded', this)
    else if (method === 'frameAttached')
      this.#onFrameAttached((params as FrameEventParams).frame)
    else if (method === 'frameDetached')
      this.#onFrameDetached((params as FrameEventParams).frame)
    else if (method === 'frameNavigated')
      this.#onFrameNavigated(params as FrameEventParams)
    else if (method === 'locatorHandlerTriggered')
      this.#onLocatorHandlerTriggered((params as { uid: number }).uid).catch(
        (error) => this.#context._pipe.report(error),
      )
  }

  /** Playwright: `client/page.ts` `Page._locatorHandlers` */
  readonly #locatorHandlers = new Map<
    number,
    {
      locator: Locator
      handler: (locator: Locator) => unknown
      times: number | undefined
    }
  >()

  /**
   * Runs `handler(locator)` whenever an action is about to act while `locator` is visible (a
   * cookie banner, a popup), then lets the action go on once it hid, unless `noWaitAfter`, at
   * most `times` times. Sent with no timeout, as Playwright's (`kNoTimeout`).
   */
  async addLocatorHandler(
    locator: Locator,
    handler: (locator: Locator) => unknown,
    options: { times?: number; noWaitAfter?: boolean } = {},
  ): Promise<void> {
    if (locator._frame !== this.#mainFrame)
      throw new Error('Locator must belong to the main frame of this page')
    if (options.times === 0) return
    // Subscribed before the handler exists, so its first trigger always reaches the realm.
    if (!this.#locatorHandlers.size)
      this._subscribeChannelEvent('locatorHandlerTriggered', true)
    try {
      const { uid } = (await this.#connection.sendPage(
        'page.addLocatorHandler',
        this.#guid,
        'registerLocatorHandler',
        pickParams(PAGE_REGISTER_LOCATOR_HANDLER_KEYS, {
          selector: locator._selector,
          noWaitAfter: options.noWaitAfter,
        }) as PageRegisterLocatorHandlerParams,
        0,
      )) as PageRegisterLocatorHandlerResult
      this.#locatorHandlers.set(uid, { locator, handler, times: options.times })
    } finally {
      if (!this.#locatorHandlers.size)
        this._subscribeChannelEvent('locatorHandlerTriggered', false)
    }
  }

  /** Playwright: `client/page.ts` `Page._onLocatorHandlerTriggered` */
  async #onLocatorHandlerTriggered(uid: number): Promise<void> {
    let remove = false
    try {
      const handler = this.#locatorHandlers.get(uid)
      if (handler && handler.times !== 0) {
        if (handler.times !== undefined) handler.times--
        await handler.handler(handler.locator)
      }
      remove = handler?.times === 0
    } finally {
      if (remove) this.#dropLocatorHandler(uid)
      this.#connection
        .sendPage(
          'page.addLocatorHandler',
          this.#guid,
          'resolveLocatorHandlerNoReply',
          pickParams(PAGE_RESOLVE_LOCATOR_HANDLER_KEYS, {
            uid,
            remove,
          }) as PageResolveLocatorHandlerNoReplyParams,
          0,
        )
        .catch(() => {})
    }
  }

  /** Removes every handler of `locator` (same frame and selector), as Playwright's. */
  async removeLocatorHandler(locator: Locator): Promise<void> {
    for (const [uid, data] of this.#locatorHandlers) {
      if (data.locator._equals(locator)) {
        this.#dropLocatorHandler(uid)
        await this.#connection
          .sendPage(
            'page.removeLocatorHandler',
            this.#guid,
            'unregisterLocatorHandler',
            pickParams(PAGE_UNREGISTER_LOCATOR_HANDLER_KEYS, {
              uid,
            }) as PageUnregisterLocatorHandlerParams,
            0,
          )
          .catch(() => {})
      }
    }
  }

  #dropLocatorHandler(uid: number): void {
    this.#locatorHandlers.delete(uid)
    if (!this.#locatorHandlers.size)
      this._subscribeChannelEvent('locatorHandlerTriggered', false)
  }

  /** Playwright's `Page._onFrameAttached`: a frame attaches with no name or URL yet. */
  #onFrameAttached(record: CreatedFrame): void {
    this.#frameEvent(record, 'frameattached', false)
  }

  /** Playwright's `Page._onFrameDetached`. */
  #onFrameDetached(record: CreatedFrame): void {
    this.#frameEvent(record, 'framedetached', true)
  }

  /** Playwright's `Frame` `navigated` handler: the frame takes the committed URL and name, then
   * the page's and the context's `framenavigated` run. */
  #onFrameNavigated({ frame, url, name }: FrameEventParams): void {
    this.#frameEvent(
      {
        guid: frame.guid,
        initializer: {
          ...frame.initializer,
          url: url ?? frame.initializer.url,
          name: name ?? frame.initializer.name,
        },
      },
      'framenavigated',
      false,
    )
  }

  /**
   * The page's listeners of a frame event, then the context's, with the Frame of the record,
   * which answers as the event describes it while they run (`Frame._during`), as Playwright's
   * client frame does: its state comes from these very events.
   */
  #frameEvent(
    record: CreatedFrame,
    event: 'frameattached' | 'framedetached' | 'framenavigated',
    detached: boolean,
  ): void {
    const frame = this._frameFrom(record)
    const { url, name } = record.initializer
    frame._during({ url, name, detached }, () => {
      this.emit(event, frame)
      this.#context.emit(event, frame)
    })
  }

  /**
   * The tab that opened this page, if it is still open in the run's context, else `null`: a
   * `newPage` page, a tab the user opened, and a page whose opener closed have none. A popup
   * opened with `noopener` names its opener, as Playwright's does on Chromium, whose target
   * reports it.
   *
   * Deviation: `client/page.ts` `Page.opener`: `if (!this._opener || this._opener.isClosed())`
   * Playwright's page keeps the opener its server named when it reported the page. Neo reads the
   * tab's opener when asked, so the run page and a `pages()` page name theirs too.
   */
  async opener(): Promise<Page | null> {
    // A closed context's tabs stay open; its pages, the opener too, read closed.
    if (this.#context.isClosed()) return null
    const record = get(this.#guid, 'opener')
    return record ? this.#context._pageFrom(record) : null
  }

  /** Playwright's `Page._onClose`: the page reads closed, then its `close` and the context's
   * `pageclose` run. Once: a context's close and its tab's close can both report it. */
  _onClose(): void {
    if (this.#closedLocally) return
    this.#closedLocally = true
    this.emit('close', this)
    this.#context.emit('pageclose', this)
  }

  /**
   * Playwright's `Page.waitForEvent`: resolves with the first `event` its predicate (sync or
   * async) accepts. It rejects at the timeout (the page's default, 30 s unless set), and when
   * the page crashes or closes first, unless that is the event awaited.
   */
  async waitForEvent(
    event: string,
    optionsOrPredicate: WaitForEventOptions = {},
  ): Promise<unknown> {
    return await wrapApiCall('page.waitForEvent', () =>
      this._waitForEvent(
        event,
        optionsOrPredicate,
        `waiting for event "${event}"`,
      ),
    )
  }

  /** Playwright's `Page._waitForEvent`, which its `waitFor*` helpers share. */
  async _waitForEvent(
    event: string,
    optionsOrPredicate: WaitForEventOptions,
    logLine?: string,
  ): Promise<unknown> {
    const options =
      typeof optionsOrPredicate === 'function' ? {} : optionsOrPredicate
    // Refuses a signal first (TimeoutSettings), inside its callers' `wrapApiCall`, which
    // prefixes the api name.
    const timeout = this._timeoutSettings.timeout(options, null)
    this._refuseUnserved(event)
    const predicate =
      typeof optionsOrPredicate === 'function'
        ? optionsOrPredicate
        : optionsOrPredicate.predicate
    const waiter = Waiter.createForEvent()
    if (logLine) waiter.log(logLine)
    waiter.rejectOnTimeout(
      timeout,
      `Timeout ${timeout}ms exceeded while waiting for event "${event}"`,
    )
    if (event !== 'crash')
      waiter.rejectOnEvent(this, 'crash', new Error('Page crashed'))
    if (event !== 'close')
      waiter.rejectOnEvent(this, 'close', () =>
        targetClosedError(this._closeReason()),
      )
    const result = await waiter.waitForEvent(this, event, predicate)
    waiter.dispose()
    return result
  }

  /**
   * The page's console messages, the last 200 as Playwright keeps them: since the last
   * main-frame navigation by default, every kept one with `filter: 'all'`. Messages the run's
   * listeners already saw are included.
   *
   * Deviation: `server/page.ts` `Page.addConsoleMessage`: `this._consoleMessages.push(message);`
   * Playwright's server keeps a page's messages and errors from the page's first moment. Neo
   * keeps them from the run's first look (a listener, or a read like this one), starting with
   * what the browser kept of the current document; the browser keeps nothing of a document the
   * page left before then, so `'all'` does not reach back past it.
   */
  async consoleMessages(
    options?: ConsoleMessagesOptions,
  ): Promise<ConsoleMessage[]> {
    const { messages } = await this.#connection.sendPage(
      'page.consoleMessages',
      this.#guid,
      'consoleMessages',
      pickParams(PAGE_CONSOLE_MESSAGES_KEYS, {
        filter: options?.filter,
      }) as PageConsoleMessagesParams,
      0,
    )
    return messages.map((message) => new ConsoleMessage(message, this, null))
  }

  async clearConsoleMessages(): Promise<void> {
    await this.#connection.sendPage(
      'page.clearConsoleMessages',
      this.#guid,
      'clearConsoleMessages',
      {},
      0,
    )
  }

  /** The page's uncaught exceptions as `Error`s, kept as {@link Page.consoleMessages} keeps
   * messages. */
  async pageErrors(options?: ConsoleMessagesOptions): Promise<Error[]> {
    const { errors } = await this.#connection.sendPage(
      'page.pageErrors',
      this.#guid,
      'pageErrors',
      pickParams(PAGE_PAGE_ERRORS_KEYS, {
        filter: options?.filter,
      }) as PagePageErrorsParams,
      0,
    )
    return errors.map((error) => parseSerializedError(error))
  }

  async clearPageErrors(): Promise<void> {
    await this.#connection.sendPage(
      'page.clearPageErrors',
      this.#guid,
      'clearPageErrors',
      {},
      0,
    )
  }

  mainFrame(): Frame {
    return this.#mainFrame
  }

  /** Interns records from every getter so frame comparisons remain valid across reads. */
  _frameFrom(record: CreatedFrame): Frame {
    let frame = this.#frames.get(record.guid)
    if (!frame) {
      frame = new Frame(record.guid, this, this.#connection, record.initializer)
      this.#frames.set(record.guid, frame)
    } else frame._updateInitializer(record.initializer)
    return frame
  }

  frames(): Frame[] {
    return get(this.#guid, 'frames').map((record) => this._frameFrom(record))
  }

  /** As Playwright, a nonempty name takes precedence when both selectors are given. */
  frame(
    frameSelector: string | { name?: string; url?: URLMatch },
  ): Frame | null {
    const name = isString(frameSelector) ? frameSelector : frameSelector.name
    const url =
      typeof frameSelector === 'object' ? frameSelector.url : undefined
    if (!name && !url)
      throw new Error('Either name or url matcher should be specified')
    const matcher = name ? undefined : serializeUrlMatch('page.frame', url)
    // Neo: Reuses one live frame-tree reply and refreshes its canonical Frame objects.
    const records = get(this.#guid, 'frames')
    const frames = records.map((record) => this._frameFrom(record))
    const index = records.findIndex(({ initializer }) =>
      matcher
        ? matchesURL(initializer.url, matcher)
        : initializer.name === name,
    )
    return frames[index] ?? null
  }

  context(): BrowserContext {
    return this.#context
  }

  setDefaultTimeout(timeout: number): void {
    this._timeoutSettings.setDefaultTimeout(timeout)
  }

  setDefaultNavigationTimeout(timeout: number): void {
    this._timeoutSettings.setDefaultNavigationTimeout(timeout)
  }

  /** The navigation's `Response`, or `null` where Playwright's has none. */
  async goto(url: string, options: GotoOptions = {}): Promise<Response | null> {
    return await this.#mainFrame._goto('page.goto', url, options)
  }

  /** Playwright's `page.$`: `frame.$` in the main frame. */
  async $(
    selector: string,
    options?: { strict?: boolean },
  ): Promise<ElementHandle | null> {
    return await this.mainFrame()._querySelector('page.$', selector, options)
  }

  /** Playwright's `page.$$`: `frame.$$` in the main frame. */
  async $$(selector: string): Promise<ElementHandle[]> {
    return await this.mainFrame()._querySelectorAll('page.$$', selector)
  }

  /** Playwright's `page.$eval`: `frame.$eval` in the main frame. */
  async $eval<R, Arg>(
    selector: string,
    pageFunction: PageFunctionOn<PageElement, Arg, R>,
    arg?: Arg,
  ): Promise<R> {
    // biome-ignore lint/complexity/noArguments: Playwright counts the arguments the call passed
    assertMaxArguments(arguments.length, 3)
    return await this.mainFrame()._evalOnSelector(
      'page.$eval',
      selector,
      pageFunction,
      arg,
    )
  }

  /** Playwright's `page.$$eval`: `frame.$$eval` in the main frame. */
  async $$eval<R, Arg>(
    selector: string,
    pageFunction: PageFunctionOn<PageElement[], Arg, R>,
    arg?: Arg,
  ): Promise<R> {
    // biome-ignore lint/complexity/noArguments: Playwright counts the arguments the call passed
    assertMaxArguments(arguments.length, 3)
    return await this.mainFrame()._evalOnSelectorAll(
      'page.$$eval',
      selector,
      pageFunction,
      arg,
    )
  }

  /** Playwright's `page.waitForFunction`: `frame.waitForFunction` in the main frame. */
  async waitForFunction<R, Arg>(
    pageFunction: PageFunction<Arg, R>,
    arg?: Arg,
    options?: WaitForFunctionOptions,
  ): Promise<JSHandle<R>> {
    return await this.mainFrame()._waitForFunction(
      'page.waitForFunction',
      pageFunction,
      arg,
      options,
    )
  }

  /** Playwright's `page.evaluate`: `frame.evaluate` in the main frame. */
  async evaluate<R, Arg>(
    pageFunction: PageFunction<Arg, R>,
    arg?: Arg,
    options?: EvaluateOptions,
  ): Promise<R> {
    // biome-ignore lint/complexity/noArguments: Playwright counts the arguments the call passed
    assertMaxArguments(arguments.length, 3)
    return await this.mainFrame()._evaluate(
      'page.evaluate',
      pageFunction,
      arg,
      options,
    )
  }

  /** Playwright's `page.evaluateHandle`: `frame.evaluateHandle` in the main frame. */
  async evaluateHandle<R, Arg>(
    pageFunction: PageFunction<Arg, R>,
    arg?: Arg,
    options?: EvaluateOptions,
  ): Promise<JSHandle<R>> {
    // biome-ignore lint/complexity/noArguments: Playwright counts the arguments the call passed
    assertMaxArguments(arguments.length, 3)
    return await this.mainFrame()._evaluateHandle(
      'page.evaluateHandle',
      pageFunction,
      arg,
      options,
    )
  }

  async title(): Promise<string> {
    return await this.#mainFrame._title('page.title')
  }

  /**
   * Reloads the page and resolves the new document's main `Response` once `waitUntil` (default
   * `load`; also `domcontentloaded` or `commit`) fires for it, as Playwright's `page.reload`.
   * A failed reload rejects with its network error. Runs under the navigation timeout.
   * Neo limit: `networkidle` throws an `Error`.
   */
  async reload(options: HistoryOptions = {}): Promise<Response | null> {
    const waitUntil = verifyLoadState(
      'waitUntil',
      options.waitUntil === undefined ? 'load' : options.waitUntil,
    )
    const result = await this.#connection.sendPage(
      'page.reload',
      this.#guid,
      'reload',
      pickParams(PAGE_RELOAD_KEYS, {
        ...options,
        waitUntil,
      }) as PageReloadParams,
      this._timeoutSettings.navigationTimeout(options, 'page.reload'),
    )
    return this.#response(result?.response)
  }

  /**
   * Navigates to the previous page in history, as Playwright's `page.goBack`: resolves `null` at
   * once when there is none; otherwise, once `waitUntil` fires, the new document's `Response`,
   * or `null` for a same-document entry or a document restored from the back-forward cache, which
   * counts as loaded. A failed navigation rejects. Options and limits as `reload`'s.
   */
  async goBack(options: HistoryOptions = {}): Promise<Response | null> {
    const waitUntil = verifyLoadState(
      'waitUntil',
      options.waitUntil === undefined ? 'load' : options.waitUntil,
    )
    const result = await this.#connection.sendPage(
      'page.goBack',
      this.#guid,
      'goBack',
      pickParams(PAGE_GO_BACK_KEYS, {
        ...options,
        waitUntil,
      }) as PageGoBackParams,
      this._timeoutSettings.navigationTimeout(options, 'page.goBack'),
    )
    return this.#response(result?.response)
  }

  /** {@link Page.goBack} toward the next page in history. */
  async goForward(options: HistoryOptions = {}): Promise<Response | null> {
    const waitUntil = verifyLoadState(
      'waitUntil',
      options.waitUntil === undefined ? 'load' : options.waitUntil,
    )
    const result = await this.#connection.sendPage(
      'page.goForward',
      this.#guid,
      'goForward',
      pickParams(PAGE_GO_FORWARD_KEYS, {
        ...options,
        waitUntil,
      }) as PageGoForwardParams,
      this._timeoutSettings.navigationTimeout(options, 'page.goForward'),
    )
    return this.#response(result?.response)
  }

  /** The `Response` a history command answered, built once per guid, or `null`. */
  #response(response: ResponseChannel | undefined): Response | null {
    return response
      ? responseFor(this.#connection, response.guid, this.#mainFrame)
      : null
  }

  /** The main frame's {@link Frame.waitForNavigation}. */
  async waitForNavigation(
    options: WaitForNavigationOptions = {},
  ): Promise<Response | null> {
    return await this.#mainFrame._waitForNavigation(
      'page.waitForNavigation',
      options,
    )
  }

  /**
   * The main frame's {@link Frame.waitForURL}: waits until the page's URL matches `url` (a glob
   * string, a `RegExp` or a predicate on its `URL`), then until `waitUntil` (default `load`;
   * `networkidle` throws) fires, at once if the current document already fired it, and resolves
   * `undefined`.
   */
  async waitForURL(
    url: NavigationURLMatch,
    options?: WaitForURLOptions,
  ): Promise<void> {
    return await this.#mainFrame._waitForURL('page.waitForURL', url, options)
  }

  /**
   * The main frame's {@link Frame.waitForSelector}: waits until `selector` meets `state`
   * (default `visible`) and resolves its first match as an {@link ElementHandle}, or `null` for a
   * `hidden` or `detached` wait, running the page's locator handlers before each attempt.
   */
  waitForSelector(
    selector: string,
    options?: WaitForSelectorOptions & { state?: 'attached' | 'visible' },
  ): Promise<ElementHandle>
  waitForSelector(
    selector: string,
    options: WaitForSelectorOptions,
  ): Promise<ElementHandle | null>
  /** Playwright: `client/page.ts` `Page.waitForSelector`: `return await this._mainFrame.waitForSelector(selector, options);` */
  async waitForSelector(
    selector: string,
    options?: WaitForSelectorOptions,
  ): Promise<ElementHandle | null> {
    return await this.#mainFrame._waitForSelectorPublic(
      'page.waitForSelector',
      selector,
      options,
    )
  }

  /**
   * The main frame's {@link Frame.waitForLoadState}: waits until `state` (default `load`;
   * `networkidle` throws) fired for the current or next document, and resolves `undefined`.
   */
  async waitForLoadState(
    state?: LifecycleEvent,
    options?: WaitForLoadStateOptions,
  ): Promise<void> {
    return await this.#mainFrame._waitForLoadState(
      'page.waitForLoadState',
      state,
      options,
    )
  }

  /** Not strict, as Playwright's `page.textContent`, which forwards to the main frame. */
  async textContent(
    selector: string,
    options: TextContentOptions = {},
  ): Promise<string | null> {
    return await this.#mainFrame._textContent(
      'page.textContent',
      selector,
      options,
    )
  }

  async innerText(
    selector: string,
    options: InnerTextOptions = {},
  ): Promise<string> {
    return await this.#mainFrame._innerText('page.innerText', selector, options)
  }

  async innerHTML(
    selector: string,
    options: InnerHTMLOptions = {},
  ): Promise<string> {
    return await this.#mainFrame._innerHTML('page.innerHTML', selector, options)
  }

  async getAttribute(
    selector: string,
    name: string,
    options: GetAttributeOptions = {},
  ): Promise<string | null> {
    return await this.#mainFrame._getAttribute(
      'page.getAttribute',
      selector,
      name,
      options,
    )
  }

  /**
   * The main frame's accessibility tree as Playwright's YAML. With `mode: 'ai'`, each rendered
   * iframe's document appears under its `iframe` node, as in Playwright, and nodes carry refs that
   * `page.locator('aria-ref=…')` resolves, in this run or a later one, until that document's next
   * snapshot or navigation. A ref inside a frame is its path from the main frame:
   * `- iframe [ref=e7]:` holds `- textbox "Email" [ref=f7e3]`, and a frame inside it adds a step
   * (`f7f2e1`); Playwright numbers frames instead (`f1e3`). The default mode prints iframes without
   * their content, as Playwright's does. `depth` limits tree levels (0 means unlimited).
   * For a section, use `locator.ariaSnapshot()`. AI mode does not auto-wait for that locator,
   * so use `locator.waitFor()` first if needed.
   */
  async ariaSnapshot(options: AriaSnapshotOptions = {}): Promise<string> {
    return await this.#mainFrame._ariaSnapshot('page.ariaSnapshot', options)
  }

  /** Not strict, as Playwright's `page.click`, which forwards to the main frame. */
  async click(selector: string, options: ClickOptions = {}): Promise<void> {
    return await this.#mainFrame._click('page.click', selector, options)
  }

  /** Not strict, as Playwright's `page.dblclick`, which forwards to the main frame. */
  async dblclick(
    selector: string,
    options: DblclickOptions = {},
  ): Promise<void> {
    return await this.#mainFrame._dblclick('page.dblclick', selector, options)
  }

  /** Not strict, as Playwright's `page.hover`, which forwards to the main frame. */
  async hover(selector: string, options: HoverOptions = {}): Promise<void> {
    return await this.#mainFrame._hover('page.hover', selector, options)
  }

  /** Not strict, as Playwright's `page.tap`, which forwards to the main frame. */
  async tap(selector: string, options: TapOptions = {}): Promise<void> {
    return await this.#mainFrame._tap('page.tap', selector, options)
  }

  async fill(
    selector: string,
    value: string,
    options: FillOptions = {},
  ): Promise<void> {
    return await this.#mainFrame._fill('page.fill', selector, value, options)
  }

  async press(
    selector: string,
    key: string,
    options: PressOptions = {},
  ): Promise<void> {
    return await this.#mainFrame._press('page.press', selector, key, options)
  }

  async inputValue(
    selector: string,
    options: InputValueOptions = {},
  ): Promise<string> {
    return await this.#mainFrame._inputValue(
      'page.inputValue',
      selector,
      options,
    )
  }

  async isVisible(
    selector: string,
    options: IsVisibleOptions = {},
  ): Promise<boolean> {
    return await this.#mainFrame._isVisible('page.isVisible', selector, options)
  }

  async isChecked(
    selector: string,
    options: IsCheckedOptions = {},
  ): Promise<boolean> {
    return await this.#mainFrame._isChecked('page.isChecked', selector, options)
  }

  async isHidden(
    selector: string,
    options: IsHiddenOptions = {},
  ): Promise<boolean> {
    return await this.#mainFrame._isHidden('page.isHidden', selector, options)
  }

  async isEnabled(
    selector: string,
    options: IsEnabledOptions = {},
  ): Promise<boolean> {
    return await this.#mainFrame._isEnabled('page.isEnabled', selector, options)
  }

  async isDisabled(
    selector: string,
    options: IsDisabledOptions = {},
  ): Promise<boolean> {
    return await this.#mainFrame._isDisabled(
      'page.isDisabled',
      selector,
      options,
    )
  }

  async isEditable(
    selector: string,
    options: IsEditableOptions = {},
  ): Promise<boolean> {
    return await this.#mainFrame._isEditable(
      'page.isEditable',
      selector,
      options,
    )
  }

  async check(selector: string, options: CheckOptions = {}): Promise<void> {
    return await this.#mainFrame._check('page.check', selector, options)
  }

  async uncheck(selector: string, options: UncheckOptions = {}): Promise<void> {
    return await this.#mainFrame._uncheck('page.uncheck', selector, options)
  }

  async selectOption(
    selector: string,
    values: SelectOptionValues,
    options: SelectOptionOptions = {},
  ): Promise<string[]> {
    return await this.#mainFrame._selectOption(
      'page.selectOption',
      selector,
      values,
      options,
    )
  }

  async setChecked(
    selector: string,
    checked: boolean,
    options: CheckOptions = {},
  ): Promise<void> {
    if (checked)
      await this.#mainFrame._check('page.setChecked', selector, options)
    else await this.#mainFrame._uncheck('page.setChecked', selector, options)
  }

  locator(selector: string, options?: LocatorOptions): Locator {
    return this.mainFrame().locator(selector, options)
  }

  /** The element an `ai` snapshot named `ref` (`page.ariaSnapshot({ mode: 'ai' })`), until a
   * later snapshot of its document or a navigation renames it. */
  getByRef(ref: string): Locator {
    return this.locator(`aria-ref=${ref}`)
  }

  frameLocator(selector?: string): FrameLocator {
    return this.mainFrame().frameLocator(selector)
  }

  getByTestId(testId: string | RegExp): Locator {
    return this.mainFrame().getByTestId(testId)
  }

  getByAltText(text: string | RegExp, options?: { exact?: boolean }): Locator {
    return this.mainFrame().getByAltText(text, options)
  }

  getByLabel(text: string | RegExp, options?: { exact?: boolean }): Locator {
    return this.mainFrame().getByLabel(text, options)
  }

  getByPlaceholder(
    text: string | RegExp,
    options?: { exact?: boolean },
  ): Locator {
    return this.mainFrame().getByPlaceholder(text, options)
  }

  getByText(text: string | RegExp, options?: { exact?: boolean }): Locator {
    return this.mainFrame().getByText(text, options)
  }

  getByTitle(text: string | RegExp, options?: { exact?: boolean }): Locator {
    return this.mainFrame().getByTitle(text, options)
  }

  getByRole(role: string, options: ByRoleOptions = {}): Locator {
    return this.mainFrame().getByRole(role, options)
  }

  /** Playwright's `ChannelOwner.toJSON`: `JSON.stringify(page)`, and a script that returns its
   * page, give its type and guid only, never this object's internals. */
  toJSON(): { _type: 'Page'; _guid: string } {
    return { _type: 'Page', _guid: this.#guid }
  }
}
