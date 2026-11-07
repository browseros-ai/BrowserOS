import type {
  LifecycleEvent,
  PageCloseParams,
  PageCommands,
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
import {
  isString,
  trimStringWithEllipsis,
} from '../vendor/playwright/isomorphic/stringUtils'
import {
  BindingCall,
  type BindingCallback,
  type BindingCallRecord,
} from './binding-call'
import type { BrowserContext } from './browser-context'
import type { Buffer } from './buffer'
import type { ApiName, Connection } from './connection'
import { ConsoleMessage } from './console-message'
import type { ElementHandle } from './element-handle'
import {
  isTargetClosedError,
  parseSerializedError,
  targetClosedError,
} from './errors'
import { ChannelOwner, type EventTable } from './events'
import { FileChooser } from './file-chooser'
import {
  type AriaSnapshotJSON,
  type AriaSnapshotOptions,
  type CheckOptions,
  type ClickOptions,
  type ContentOptions,
  type DblclickOptions,
  type DispatchEventOptions,
  type DragAndDropOptions,
  type FillOptions,
  type FocusOptions,
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
  type SetContentOptions,
  type TagOptions,
  type TapOptions,
  type TextContentOptions,
  type TypeOptions,
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
import { type Headers, headersObjectToArray, validateHeaders } from './headers'
import { Disposable, evaluationScript, type InitScript } from './init-script'
import { Keyboard, Mouse, Touchscreen } from './input'
import {
  assertEvaluateOptions,
  assertMaxArguments,
  type EvaluateOptions,
  type JSHandle,
  type PageElement,
  type PageFunction,
  type PageFunctionOn,
} from './js-handle'
import type { Locator, LocatorOptions } from './locator'
import { isRegExp, serializeUrlMatch } from './neo-commands'
import {
  networkObject,
  type Request,
  type Response,
  responseFor,
} from './network'
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
import {
  type PageScreenshotOptions,
  type PDFOptions,
  pageScreenshotParams,
  pdfParams,
  resultBuffer,
} from './screenshot'
import type { SelectOptionValues } from './select-option'
import { type TimeoutInput, TimeoutSettings } from './timeout-settings'
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
    'filechooser',
    'download',
    'request',
    'response',
    'requestfailed',
    'requestfinished',
  ],
  quiet: [],
  unserved: ['websocket', 'worker'],
}

/** `waitForRequest` and `waitForResponse`'s options: Playwright's `TimeoutOptions`; the Waiter
 * honours `signal` (`_waitForEvent`). */
export type WaitForNetworkOptions = TimeoutInput

/** Playwright: `client/page.ts` `trimUrl` */
function trimUrl(param: unknown): string | undefined {
  if (isRegExp(param))
    return `/${trimStringWithEllipsis(param.source, 50)}/${param.flags}`
  if (isString(param)) return `"${trimStringWithEllipsis(param, 50)}"`
  return undefined
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
  /** Playwright's `page.keyboard`, `page.mouse` and `page.touchscreen`, one each per page. */
  readonly keyboard: Keyboard
  readonly mouse: Mouse
  readonly touchscreen: Touchscreen
  /** Set when this page's `close` ran (`_onClose`): what Playwright's client flips then. */
  #closedLocally = false
  /** Playwright's `_closeReason`: what `close({ reason })` was given. */
  #closeReason: string | undefined
  /** The page's exposed callbacks, by name (Playwright's `Page._bindings`). */
  readonly _bindings = new Map<string, BindingCallback>()

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
    this.keyboard = new Keyboard(this)
    this.mouse = new Mouse(this)
    this.touchscreen = new Touchscreen(this)
  }

  /** Sends a device command to this page's tab, with no timeout (Playwright's `kNoTimeout`). */
  async _sendInput<M extends keyof PageCommands>(
    apiName: ApiName,
    method: M,
    params: PageCommands[M]['params'],
  ): Promise<void> {
    await this.#connection.sendPage(apiName, this.#guid, method, params, 0)
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

  /** Playwright: `client/page.ts` `Page.setExtraHTTPHeaders`: `validateHeaders(headers);`
   * Sent with every request of this tab, over the context's, until the run ends. */
  async setExtraHTTPHeaders(headers: Headers): Promise<void> {
    validateHeaders(headers)
    await this.#connection.sendPage(
      'page.setExtraHTTPHeaders',
      this.#guid,
      'setExtraHTTPHeaders',
      { headers: headersObjectToArray(headers) },
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
    else if (method === 'bindingCall')
      void this._onBinding((params as { binding: BindingCallRecord }).binding)
    else if (method === 'locatorHandlerTriggered')
      this.#onLocatorHandlerTriggered((params as { uid: number }).uid).catch(
        (error) => this.#context._pipe.report(error),
      )
    else if (method === 'fileChooser') {
      // Playwright: `client/page.ts` `Page.constructor`: `this._channel.on('fileChooser', ({ element, isMultiple }) => this.emit(Events.Page.FileChooser, new FileChooser(this, ElementHandle.from(element), isMultiple)));`
      const { chooser, isMultiple } = params as {
        chooser: string
        isMultiple: boolean
      }
      this.emit(
        'filechooser',
        new FileChooser(this, chooser, isMultiple, this.#connection),
      )
    }
  }

  /** Playwright: `client/page.ts` `Page._onBinding`: `await this._browserContext._onBinding(bindingCall);` */
  async _onBinding(record: BindingCallRecord): Promise<void> {
    const call = new BindingCall(
      this.#connection,
      record,
      this._frameForGuid(record.frame.guid),
    )
    const func = this._bindings.get(record.name)
    if (func) {
      await call.call(func)
      return
    }
    await this.#context._onBinding(call)
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
    // Subscribed before the handler exists, so its first trigger always reaches the realm, and
    // kept while its registration is pending, so a concurrent removal of the last other handler
    // cannot unsubscribe it.
    this.#pendingLocatorHandlers++
    this.#syncLocatorHandlerSubscription()
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
      this.#pendingLocatorHandlers--
      this.#syncLocatorHandlerSubscription()
    }
  }

  /** Registrations sent and not yet answered. */
  #pendingLocatorHandlers = 0
  #locatorHandlerSubscribed = false

  /** Neo: the page listens for `locatorHandlerTriggered` while it holds or registers a handler. */
  #syncLocatorHandlerSubscription(): void {
    const wanted = this.#locatorHandlers.size + this.#pendingLocatorHandlers > 0
    if (wanted === this.#locatorHandlerSubscribed) return
    this.#locatorHandlerSubscribed = wanted
    this._subscribeChannelEvent('locatorHandlerTriggered', wanted)
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
    this.#syncLocatorHandlerSubscription()
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

  /**
   * Playwright's `Page.waitForRequest`: resolves with the first request of the page whose URL
   * matches `urlOrPredicate` (a glob string, a `RegExp`), or that a predicate (sync or async)
   * accepts, from the call on. It rejects at the timeout (the page's default, 30 s unless set)
   * and when the page closes or crashes first. A glob or `RegExp` matches as `page.waitForURL`'s
   * does (neo-runtime `url_match.rs`); Neo has no `baseURL`, so a relative glob matches as
   * written.
   */
  async waitForRequest(
    urlOrPredicate:
      | URLMatch
      | ((request: Request) => boolean | Promise<boolean>),
    options: WaitForNetworkOptions = {},
  ): Promise<Request> {
    return await wrapApiCall('page.waitForRequest', async () => {
      const predicate = async (request: Request) => {
        if (isString(urlOrPredicate) || isRegExp(urlOrPredicate))
          return matchesURL(
            request.url(),
            serializeUrlMatch('page.waitForRequest', urlOrPredicate),
          )
        return await urlOrPredicate(request)
      }
      const trimmedUrl = trimUrl(urlOrPredicate)
      const logLine = trimmedUrl
        ? `waiting for request ${trimmedUrl}`
        : undefined
      return (await this._waitForEvent(
        'request',
        { predicate, timeout: options.timeout, signal: options.signal },
        logLine,
      )) as Request
    })
  }

  /** Playwright's `Page.waitForResponse`: {@link Page.waitForRequest} for responses. */
  async waitForResponse(
    urlOrPredicate:
      | URLMatch
      | ((response: Response) => boolean | Promise<boolean>),
    options: WaitForNetworkOptions = {},
  ): Promise<Response> {
    return await wrapApiCall('page.waitForResponse', async () => {
      const predicate = async (response: Response) => {
        if (isString(urlOrPredicate) || isRegExp(urlOrPredicate))
          return matchesURL(
            response.url(),
            serializeUrlMatch('page.waitForResponse', urlOrPredicate),
          )
        return await urlOrPredicate(response)
      }
      const trimmedUrl = trimUrl(urlOrPredicate)
      const logLine = trimmedUrl
        ? `waiting for response ${trimmedUrl}`
        : undefined
      return (await this._waitForEvent(
        'response',
        { predicate, timeout: options.timeout, signal: options.signal },
        logLine,
      )) as Response
    })
  }

  /**
   * Playwright's `Page.requests`: the page's most recent requests (about the last 100, each
   * redirect hop one), oldest first, as the same objects its events hand out.
   *
   * Deviation: `server/page.ts` `Page.addNetworkRequest`: `this._networkRequests.push(request);`
   * Playwright's server records a page's requests from the page's first moment; Neo records them
   * from the run's first look at the page's network (a goto, a history step or navigation wait, a
   * network listener, or this call), as Chromium replays no earlier request.
   */
  async requests(): Promise<Request[]> {
    const { requests } = await this.#connection.sendPage(
      'page.requests',
      this.#guid,
      'requests',
      {},
      0,
    )
    return requests.map(
      (request) =>
        networkObject(this.#connection, request.guid, this) as Request,
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
    // Inside its callers' `wrapApiCall`, which prefixes the api name.
    const timeoutOptions = this._timeoutSettings.timeout(options, null)
    this._refuseUnserved(event)
    const predicate =
      typeof optionsOrPredicate === 'function'
        ? optionsOrPredicate
        : optionsOrPredicate.predicate
    const waiter = Waiter.createForEvent()
    if (logLine) waiter.log(logLine)
    waiter.rejectOnTimeout(
      timeoutOptions,
      `Timeout ${timeoutOptions.timeout}ms exceeded while waiting for event "${event}"`,
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

  /**
   * The frame a request's or a binding call's record names, by guid: the object this page
   * handed out for it (the one it held when that frame detached included, as Playwright's client
   * resolves a call's frame channel to its object), or a new one, which reads its url and name
   * live like any other.
   *
   * Playwright: `server/page.ts` `PageBinding.dispatch`: `const result = await binding.playwrightFunction({ frame: context.frame, page, context: page.browserContext }, ...args);`
   */
  _frameForGuid(guid: string | undefined): Frame {
    if (!guid) return this.#mainFrame
    let frame = this.#frames.get(guid)
    if (!frame) {
      frame = new Frame(guid, this, this.#connection)
      this.#frames.set(guid, frame)
    }
    return frame
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

  /** Playwright's `page.exposeFunction`: `window[name](...args)` in every frame of the page calls
   * `callback(...args)` in the run and answers with its result. `dispose()` removes it. */
  async exposeFunction(
    name: string,
    // biome-ignore lint/suspicious/noExplicitAny: Playwright's callback type.
    callback: (...args: any[]) => unknown,
  ): Promise<Disposable> {
    return await this.#expose('page.exposeFunction', name, (_source, ...args) =>
      callback(...args),
    )
  }

  /** Playwright's `page.exposeBinding`: as `exposeFunction`, with the call's
   * `{ context, page, frame }` first. */
  async exposeBinding(
    name: string,
    callback: BindingCallback,
  ): Promise<Disposable> {
    return await this.#expose('page.exposeBinding', name, callback)
  }

  async #expose(
    apiName: ApiName,
    name: string,
    callback: BindingCallback,
  ): Promise<Disposable> {
    // Deviation: `client/page.ts` `Page.exposeBinding`: `this._bindings.set(name, callback);` Playwright sets the callback after its answer; Neo before, as the answer's events, a first page call among them, run before the caller goes on.
    const previous = this._bindings.get(name)
    this._bindings.set(name, callback)
    this._subscribeChannelEvent('bindingCall', true)
    try {
      const result = await this.#connection.sendBinding(
        apiName,
        this.#guid,
        'exposeBinding',
        { name },
        0,
      )
      return new Disposable(this.#connection, result.disposable.guid)
    } catch (error) {
      if (previous) this._bindings.set(name, previous)
      else this._bindings.delete(name)
      throw error
    }
  }

  /** Playwright's `page.addInitScript`: `script` (a function called with `arg`, source text, or
   * `{ content }`) runs before the page's own scripts in every new document, in its main
   * world, out-of-process iframes included. The answer's `dispose()` removes it again. */
  async addInitScript(
    script: InitScript,
    arg?: unknown,
    options?: EvaluateOptions,
  ): Promise<Disposable> {
    assertEvaluateOptions(options)
    const signal = (options as { signal?: unknown } | undefined)?.signal
    if (signal !== undefined && signal !== null)
      // Deviation: `client/page.ts` `Page.addInitScript`: `return DisposableObject.from((await this._channel.addInitScript({ source }, kNoTimeout)).disposable);` Playwright sends no signal and ignores one; Neo refuses it.
      throw new Error(`page.addInitScript: Neo does not support signal yet`)
    if (options?.exposeFunctions)
      // Deviation: `client/page.ts` `Page.addInitScript`: `return await addInitScriptWithExposedFunctions(this, script, arg);` Neo has no page-to-realm callback yet.
      throw new Error(
        `page.addInitScript: Neo does not support exposeFunctions yet`,
      )
    const source = evaluationScript('page.addInitScript', script, arg)
    const result = await this.#connection.sendInit(
      'page.addInitScript',
      this.#guid,
      'addInitScript',
      { source },
      0,
    )
    return new Disposable(this.#connection, result.disposable.guid)
  }

  /** Playwright's `page.addScriptTag`: `frame.addScriptTag` in the main frame. */
  async addScriptTag(
    options: TagOptions & { type?: string } = {},
  ): Promise<ElementHandle> {
    return await this.mainFrame()._addTag(
      'page.addScriptTag',
      'addScriptTag',
      options,
    )
  }

  /** Playwright's `page.addStyleTag`: `frame.addStyleTag` in the main frame. */
  async addStyleTag(options: TagOptions = {}): Promise<ElementHandle> {
    return await this.mainFrame()._addTag(
      'page.addStyleTag',
      'addStyleTag',
      options,
    )
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
   * `load`; also `domcontentloaded`, `networkidle` or `commit`) fires for it, as Playwright's
   * `page.reload`. A failed reload rejects with its network error. Runs under the navigation
   * timeout.
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
   * string, a `RegExp` or a predicate on its `URL`), then until `waitUntil` (default `load`)
   * fires, at once if the current document already fired it, and resolves `undefined`.
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
   * The main frame's {@link Frame.waitForLoadState}: waits until `state` (default `load`) fired
   * for the current or next document, and resolves `undefined`.
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
  /**
   * Playwright's `Page.screenshot`: the viewport, the full page, or a clip of either, as PNG,
   * JPEG or WebP bytes. `path` (relative to the run's artifact directory) also writes them
   * there (`screenshot.ts`).
   */
  async screenshot(options: PageScreenshotOptions = {}): Promise<Buffer> {
    const result = await this.#connection.sendPage(
      'page.screenshot',
      this.#guid,
      'screenshot',
      pageScreenshotParams('page.screenshot', options),
      this._timeoutSettings.timeout(options, 'page.screenshot'),
    )
    return resultBuffer(result.binary)
  }

  /**
   * Playwright's `Page.pdf`: the page printed as PDF bytes, with no timeout, as Playwright's
   * client calls it. `path` (relative to the run's artifact directory) also writes them there.
   */
  async pdf(options: PDFOptions = {}): Promise<Buffer> {
    const result = await this.#connection.sendPage(
      'page.pdf',
      this.#guid,
      'pdf',
      pdfParams(options),
      0,
    )
    return resultBuffer(result.pdf)
  }

  async ariaSnapshot(options: AriaSnapshotOptions = {}): Promise<string> {
    return await this.#mainFrame._ariaSnapshot('page.ariaSnapshot', options)
  }

  /** The tree `ariaSnapshot` renders, as Playwright's `AriaSnapshotJSON`. */
  async ariaSnapshotJSON(
    options: AriaSnapshotOptions = {},
  ): Promise<AriaSnapshotJSON> {
    return await this.#mainFrame._ariaSnapshotJSON(
      'page.ariaSnapshotJSON',
      options,
    )
  }

  /** The main frame's markup, as Playwright's `page.content`. */
  async content(options?: ContentOptions): Promise<string> {
    return await this.#mainFrame._content('page.content', options)
  }

  /** Writes `html` into the main frame, as Playwright's `page.setContent`. */
  async setContent(html: string, options?: SetContentOptions): Promise<void> {
    return await this.#mainFrame._setContent('page.setContent', html, options)
  }

  /** Waits `timeout` milliseconds in the main frame, as Playwright's `page.waitForTimeout`. */
  async waitForTimeout(timeout: number): Promise<void> {
    return await this.#mainFrame._waitForTimeout('page.waitForTimeout', timeout)
  }

  /** Not strict, as Playwright's `page.click`, which forwards to the main frame. */
  async click(selector: string, options: ClickOptions = {}): Promise<void> {
    return await this.#mainFrame._click('page.click', selector, options)
  }

  /** Not strict, as Playwright's `page.focus`, which forwards to the main frame. */
  async focus(selector: string, options: FocusOptions = {}): Promise<void> {
    return await this.#mainFrame._focus('page.focus', selector, options)
  }

  /** Not strict, as Playwright's `page.dispatchEvent`, which forwards to the main frame. */
  async dispatchEvent(
    selector: string,
    type: string,
    eventInit?: unknown,
    options: DispatchEventOptions = {},
  ): Promise<void> {
    return await this.#mainFrame._dispatchEvent(
      'page.dispatchEvent',
      selector,
      type,
      eventInit,
      options,
    )
  }

  /** Hides every highlight of this page, in every frame. Sent with no timeout, as Playwright's
   * (`kNoTimeout`). */
  async hideHighlight(): Promise<void> {
    await this.#connection.sendPage(
      'page.hideHighlight',
      this.#guid,
      'hideHighlight',
      {},
      0,
    )
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

  async dragAndDrop(
    source: string,
    target: string,
    options: DragAndDropOptions = {},
  ): Promise<void> {
    return await this.#mainFrame._dragAndDrop(
      'page.dragAndDrop',
      source,
      target,
      options,
    )
  }

  /** Deprecated in Playwright for `Locator.pressSequentially`, and served the same way. */
  async type(
    selector: string,
    text: string,
    options: TypeOptions = {},
  ): Promise<void> {
    return await this.#mainFrame._type('page.type', selector, text, options)
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
