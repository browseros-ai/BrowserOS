/**
 * Playwright's `BrowserContext` (`client/browserContext.ts` at 26137face), cut to what a run
 * needs: its pages, `newPage`, the default timeout its pages fall back to, and the events Rust
 * sends on it (`page`, `console`, `pageError`, `dialog`, `dialogClosed`, `request`, `response`,
 * `requestFinished`, `requestFailed`), which it fans out to the page each names and to its own
 * listeners in Playwright's order. Its `pageload`, `pageclose` and frame events come from its
 * pages. Scripts receive it as `context`.
 *
 * Its page-level listeners cover the pages the run has a session on, which include every page a
 * `page` event handed out: Neo does not attach to the user's other tabs just to listen
 * (med-pw-net design, Decisions).
 *
 * Playwright keeps its page list current from the server's `page` and `close` events. Neo sends
 * `page` only while the run listens for new pages, so `pages()` reads the context's open tabs
 * live, and this class keeps one `Page` per guid across reads and events. That map is what
 * makes a page the same object every time it comes back (`===`, `includes`, `indexOf`, the
 * `page` event's page and the `popup` event's), and what binds each `Page` to its own tab: a
 * `Page` and its main frame are built once, from the record Rust minted their guids in.
 */

import type {
  BrowserContextCloseParams,
  DialogInitializer,
  PageChannel,
  RequestInitializer,
  ResponseInitializer,
  SerializedError,
} from '@browseros/neo-runtime-spec/serve'
import type { Connection } from './connection'
import { ConsoleMessage, type ConsoleMessagePayload } from './console-message'
import { Dialog } from './dialog'
import { DisposableStub } from './disposable'
import { parseSerializedError, targetClosedError } from './errors'
import {
  ChannelOwner,
  type EventPipe,
  type EventTable,
  reportUncaught,
} from './events'
import { type Created, requestFromRecord, responseFromRecord } from './network'
import { Page } from './page'
import { BROWSER_CONTEXT_CLOSE_KEYS, pickParams } from './protocol'
import {
  Route,
  RouteHandler,
  type RouteHandlerCallback,
  type RouteURLMatch,
  type UnrouteBehavior,
  urlMatchesEqual,
} from './route'
import { type CreatedPage, get } from './runtime-bridge'
import { TimeoutSettings } from './timeout-settings'
import { Waiter, type WaitForEventOptions, wrapApiCall } from './waiter'
import { WebError, type WebErrorLocation } from './web-error'

/**
 * Neo: the context's listener events, by Playwright's names (`Events.BrowserContext`). `close`
 * never happens: the run's context is the user's browser, which outlives the run.
 */
const CONTEXT_EVENTS: EventTable = {
  receiver: 'BrowserContext',
  served: [
    'console',
    'weberror',
    'dialog',
    'dialogclosed',
    'request',
    'response',
    'requestfailed',
    'requestfinished',
    'page',
    'pageclose',
    'pageload',
    'frameattached',
    'framedetached',
    'framenavigated',
  ],
  local: ['close'],
  quiet: ['backgroundpage'],
  unserved: ['download', 'serviceworker'],
}

/** A dialog record Rust sends: Playwright's `{guid, initializer}`. */
interface DialogRecord {
  guid: string
  initializer: DialogInitializer
}

export class BrowserContext extends ChannelOwner {
  /** `context@default`, the guid Rust serves this context's getters and calls on. */
  readonly #guid: string
  readonly #connection: Connection
  /** Playwright's `BrowserContext._routes`: the context's route handlers, newest first. */
  _routes: RouteHandler[] = []
  /** Whether the client listens to the context's `route` channel event (`_listenToChannel`). */
  #routeListening = false
  /** The last pattern update sent, which the next one waits for. */
  #patternsSent: Promise<void> = Promise.resolve()
  /**
   * By guid, every page the run was handed: the run page, every `pages()` record, every
   * `newPage` result. An entry stays for the whole run, so a tab that one read leaves out and
   * a later read lists again is still the same object, as in Playwright, where only a close
   * takes a page off the list. The map grows only with the tabs one run sees. Without it, each
   * `pages()` call would return new objects.
   */
  readonly #pages = new Map<string, Page>()
  /** Every dialog the run's events handed out, by guid: one object per dialog. */
  readonly #dialogs = new Map<string, Dialog>()
  readonly _timeoutSettings = new TimeoutSettings()
  /** The run's event pipe, which this context's pages share. */
  readonly _pipe: EventPipe
  /** Playwright's `_closingStatus`: `closing` from `close()`'s call, `closed` once done. */
  #closingStatus: 'none' | 'closing' | 'closed' = 'none'
  #closeReason: string | undefined

  constructor(guid: string, connection: Connection, pipe: EventPipe) {
    super(guid, pipe, CONTEXT_EVENTS)
    this.#guid = guid
    this.#connection = connection
    this._pipe = pipe
    connection._closeReasonFor = (callGuid) =>
      this.#pages.get(pageGuidOf(callGuid))?._closeReason() ?? this.#closeReason
    connection._closedFor = (callGuid) =>
      this.#pages.get(pageGuidOf(callGuid))?._isClosedLocally() ?? false
  }

  /** Playwright's `BrowserContext.isClosed`: true from the moment `close()` is called. */
  isClosed(): boolean {
    return this.#closingStatus !== 'none'
  }

  /** Playwright's `_effectiveCloseReason`: the reason `close()` was given. */
  _closeReason(): string | undefined {
    return this.#closeReason
  }

  /**
   * Playwright's `close` of a connectOverCDP default context, which only disconnects: no tab
   * closes (measured on BrowserOS neo with pinned Playwright: every tab stays open, the one its
   * own client opened too). The run lets go of its listeners in the browser (Rust), then every
   * page the context lists, or the run holds, closes as the script sees it: its `close`, then
   * the context's `pageclose`; then the context's `close`. From then on `pages()` is empty,
   * every page reads closed, calls fail as closed, and the waits and calls the close
   * interrupted reject with `reason` when given, and Rust stops those calls before the close
   * resolves, so none writes input after it. A second call resolves at once; a close that
   * failed leaves the context open.
   */
  async close(options: { reason?: string } = {}): Promise<void> {
    if (this.isClosed()) return
    this.#closeReason = options.reason
    this.#closingStatus = 'closing'
    try {
      await this.#connection.sendContext(
        'browserContext.close',
        this.#guid,
        'close',
        pickParams(BROWSER_CONTEXT_CLOSE_KEYS, {
          reason: options.reason,
        }) as BrowserContextCloseParams,
        0,
      )
    } catch (error) {
      // Never half-closed: a close Rust did not do leaves the context open, to close again.
      this.#closingStatus = 'none'
      this.#closeReason = undefined
      throw error
    }
    this.#connection._close(options.reason)
    const pages = this.pages()
    for (const page of this.#pages.values())
      if (!pages.includes(page) && !page._isClosedLocally()) pages.push(page)
    for (const page of pages) page._onClose()
    this.#closingStatus = 'closed'
    this.emit('close', this)
  }

  /** Playwright's context channel handlers (`client/browserContext.ts` constructor). */
  _onEvent(method: string, params: unknown): void {
    if (method === 'page') this.#onPage((params as { page: CreatedPage }).page)
    else if (method === 'console') this.#onConsole(params as ConsoleEventParams)
    else if (method === 'pageError')
      this.#onPageError(params as PageErrorEventParams)
    else if (method === 'dialog')
      this.#onDialog((params as { dialog: DialogRecord }).dialog)
    else if (method === 'dialogClosed')
      this.#onDialogClosed((params as { dialog: DialogRecord }).dialog)
    else if (method === 'request') this.#onRequest(params as RequestEventParams)
    else if (method === 'response')
      this.#onResponse(params as ResponseEventParams)
    else if (method === 'requestFinished')
      this.#onRequestFinished(params as RequestEndEventParams)
    else if (method === 'requestFailed')
      this.#onRequestFailed(params as RequestEndEventParams)
    else if (method === 'route') this.#onRouteEvent(params as RouteEventParams)
  }

  /** The context's `route` event: a request only a context route names. */
  #onRouteEvent({ route, request, page }: RouteEventParams): void {
    const pageObject = this.#pageFor(page.guid)
    const routed = new Route(
      route.guid,
      requestFromRecord(this.#connection, request, pageObject),
      this.#connection,
      this._timeoutSettings,
    )
    this._onRoute(routed, pageObject).catch(reportUncaught)
  }

  /** Playwright's `BrowserContext._onRoute`: the context's handlers, newest first, then the
   * network, with what their `fallback()` calls changed. */
  async _onRoute(route: Route, page?: Page): Promise<void> {
    const routeHandlers = this._routes
    for (const routeHandler of routeHandlers.slice()) {
      // A page or context whose close ran stalls its requests at once, as Playwright's does.
      if (page?._isClosedLocally() || this.isClosed()) return
      if (!routeHandler.matches(route.request().url())) continue
      const index = this._routes.indexOf(routeHandler)
      if (index === -1) continue
      if (routeHandler.willExpire()) this._routes.splice(index, 1)
      const handled = await routeHandler.handle(route)
      // unrouteAll() replaces _routes and updates the patterns after waiting for handlers.
      if (routeHandlers === this._routes && !this._routes.length)
        this.#updateInterceptionPatterns().catch(() => {})
      if (handled) return
    }
    // A closed page, or an unrouteAll() that did not wait, makes this throw: silenced.
    await route._innerContinue(true).catch(() => {})
  }

  /** Playwright's `BrowserContext.route`: for every page of the run's context. */
  async route(
    url: RouteURLMatch,
    handler: RouteHandlerCallback,
    options: { times?: number } = {},
  ): Promise<DisposableStub> {
    // A bad glob throws here, prefixed; the call's own errors come back prefixed already.
    const routeHandler = await wrapApiCall(
      'browserContext.route',
      async () => new RouteHandler(url, handler, options.times),
    )
    this._routes.unshift(routeHandler)
    await this.#updateInterceptionPatterns()
    return new DisposableStub(() => this.unroute(url, handler))
  }

  /** Playwright's `BrowserContext.unrouteAll`. */
  async unrouteAll(options?: { behavior?: UnrouteBehavior }): Promise<void> {
    await this.#unrouteInternal(this._routes, [], options?.behavior)
  }

  /** Playwright's `BrowserContext.unroute`. */
  async unroute(
    url: RouteURLMatch,
    handler?: RouteHandlerCallback,
  ): Promise<void> {
    const removed: RouteHandler[] = []
    const remaining: RouteHandler[] = []
    for (const route of this._routes) {
      if (
        urlMatchesEqual(route.url, url) &&
        (!handler || route.handler === handler)
      )
        removed.push(route)
      else remaining.push(route)
    }
    await this.#unrouteInternal(removed, remaining, 'default')
  }

  async #unrouteInternal(
    removed: RouteHandler[],
    remaining: RouteHandler[],
    behavior?: UnrouteBehavior,
  ): Promise<void> {
    this._routes = remaining
    if (behavior && behavior !== 'default')
      await Promise.all(removed.map((handler) => handler.stop(behavior)))
    await this.#updateInterceptionPatterns()
  }

  /** Playwright's `BrowserContext._updateInterceptionPatterns`; the client listens to the
   * context's `route` event while it has routes. The updates queue, as the page's do. */
  async #updateInterceptionPatterns(): Promise<void> {
    const update = this.#patternsSent.then(() =>
      this.#sendInterceptionPatterns(),
    )
    this.#patternsSent = update.catch(() => {})
    await update
  }

  async #sendInterceptionPatterns(): Promise<void> {
    const patterns = RouteHandler.prepareInterceptionPatterns(this._routes)
    const listening = patterns.length > 0
    if (listening && !this.#routeListening) {
      this.#routeListening = true
      this._listenToChannel('route', true)
    }
    await this.#connection.sendContext(
      'browserContext.route',
      this._guid,
      'setNetworkInterceptionPatterns',
      { patterns },
      0,
    )
    if (!this._routes.length && this.#routeListening) {
      this.#routeListening = false
      this._listenToChannel('route', false)
    }
  }

  /** Playwright's `BrowserContext._onRequest`: the context's listeners, then the page's. */
  #onRequest({ request, page }: RequestEventParams): void {
    const pageObject = this.#pageFor(page.guid)
    const requestObject = requestFromRecord(
      this.#connection,
      request,
      pageObject,
    )
    this.emit('request', requestObject)
    pageObject.emit('request', requestObject)
  }

  /** Playwright's `BrowserContext._onResponse`. */
  #onResponse({ response, request, page }: ResponseEventParams): void {
    const pageObject = this.#pageFor(page.guid)
    const responseObject = responseFromRecord(
      this.#connection,
      response,
      requestFromRecord(this.#connection, request, pageObject),
    )
    this.emit('response', responseObject)
    pageObject.emit('response', responseObject)
  }

  /**
   * Playwright's `BrowserContext._onRequestFinished`. Its `responseEndTiming` and the response's
   * `finished()` are Rust's record, which `timing()` and `finished()` read live.
   */
  #onRequestFinished({ request, page }: RequestEndEventParams): void {
    const pageObject = this.#pageFor(page.guid)
    const requestObject = requestFromRecord(
      this.#connection,
      request,
      pageObject,
    )
    this.emit('requestfinished', requestObject)
    pageObject.emit('requestfinished', requestObject)
  }

  /** Playwright's `BrowserContext._onRequestFailed`; `failure()` reads Rust's record live. */
  #onRequestFailed({ request, page }: RequestEndEventParams): void {
    const pageObject = this.#pageFor(page.guid)
    const requestObject = requestFromRecord(
      this.#connection,
      request,
      pageObject,
    )
    this.emit('requestfailed', requestObject)
    pageObject.emit('requestfailed', requestObject)
  }

  /**
   * Playwright's `BrowserContext._onPage`: the context's `page`, then the opener's `popup`. Rust
   * names the opener only when the run knows it and has not seen it close, the check Playwright
   * makes here.
   */
  #onPage(created: CreatedPage): void {
    const page = this._pageFrom(created)
    this.emit('page', page)
    const opener = created.initializer.opener
      ? this.#pages.get(created.initializer.opener.guid)
      : undefined
    if (opener) opener.emit('popup', page)
  }

  #onConsole(event: ConsoleEventParams): void {
    const page = event.page ? this.#pageFor(event.page.guid) : null
    const consoleMessage = new ConsoleMessage(event, page, null)
    page?.emit('console', consoleMessage)
    this.emit('console', consoleMessage)
  }

  #onPageError({ error, page, location }: PageErrorEventParams): void {
    const pageObject = this.#pageFor(page.guid)
    const parsedError = parseSerializedError(error)
    this.emit('weberror', new WebError(pageObject, parsedError, location))
    if (pageObject) pageObject.emit('pageerror', parsedError)
  }

  #onDialog(record: DialogRecord): void {
    const dialogObject = this.#dialogFrom(record)
    let hasListeners = this.emit('dialog', dialogObject)
    const page = dialogObject.page()
    if (page) hasListeners = page.emit('dialog', dialogObject) || hasListeners
    // Playwright answers here too: its listener may have gone between its subscription and the
    // dialog (a `once` listener of an earlier dialog, an `off` in flight).
    if (!hasListeners) dialogObject._answerDefault()
  }

  #onDialogClosed(record: DialogRecord): void {
    const dialogObject = this.#dialogFrom(record)
    this.emit('dialogclosed', dialogObject)
    const page = dialogObject.page()
    if (page) page.emit('dialogclosed', dialogObject)
  }

  #dialogFrom(record: DialogRecord): Dialog {
    let dialog = this.#dialogs.get(record.guid)
    if (!dialog) {
      const page = record.initializer.page
        ? this.#pageFor(record.initializer.page.guid)
        : null
      dialog = new Dialog(
        record.guid,
        record.initializer,
        page,
        this.#connection,
      )
      this.#dialogs.set(record.guid, dialog)
    }
    return dialog
  }

  /**
   * Playwright's `BrowserContext.waitForEvent`: resolves with the first `event` its predicate
   * accepts, or rejects at the timeout (the context's default, 30 s unless set). The context
   * never closes during a run, so only the timeout or the predicate ends it otherwise.
   */
  async waitForEvent(
    event: string,
    optionsOrPredicate: WaitForEventOptions = {},
  ): Promise<unknown> {
    return await wrapApiCall('browserContext.waitForEvent', async () => {
      const options =
        typeof optionsOrPredicate === 'function' ? {} : optionsOrPredicate
      // Inside `wrapApiCall`, which prefixes the api name.
      const timeoutOptions = this._timeoutSettings.timeout(options, null)
      this._refuseUnserved(event)
      const predicate =
        typeof optionsOrPredicate === 'function'
          ? optionsOrPredicate
          : optionsOrPredicate.predicate
      const waiter = Waiter.createForEvent()
      waiter.rejectOnTimeout(
        timeoutOptions,
        `Timeout ${timeoutOptions.timeout}ms exceeded while waiting for event "${event}"`,
      )
      if (event !== 'close')
        waiter.rejectOnEvent(this, 'close', () =>
          targetClosedError(this.#closeReason),
        )
      const result = await waiter.waitForEvent(this, event, predicate)
      waiter.dispose()
      return result
    })
  }

  /**
   * Every open tab of the context, in the order Rust first saw them, the run page first; a
   * page seen before keeps its object. A live read, like every Neo getter, so it can list a new
   * tab before that tab's `page` event has run; after `close()`, nothing.
   *
   * Deviation: `client/browserContext.ts` `BrowserContext.pages`: `return [...this._pages];`
   * Playwright lists the pages its client was told of, in that order. Neo lists the context's
   * tabs as the browser reports them, the run page first.
   */
  pages(): Page[] {
    if (this.#closingStatus === 'closed') return []
    return get(this.#guid, 'pages')
      .map((created) => this.#pages.get(created.guid) ?? this.#add(created))
      .filter((page) => !page._isClosedLocally())
  }

  /**
   * Opens a tab on `about:blank` and resolves with its `Page` once the tab answers that URL; a
   * `page` listener has run for it by then.
   *
   * Deviation: `server/chromium/crBrowser.ts` `CRBrowserContext.doCreateNewPage`: `const { targetId } = await this._browser._session.send('Target.createTarget', { url: 'about:blank', browserContextId: this._browserContextId });`
   * Playwright's new page takes the window's focus. Neo opens it in the background of the run
   * page's window, by design: the user's active tab stays active until a script calls
   * `bringToFront`.
   */
  async newPage(): Promise<Page> {
    const { page } = await this.#connection.sendContext(
      'browserContext.newPage',
      this.#guid,
      'newPage',
      {},
      0,
    )
    return this.#pageFor(page.guid)
  }

  setDefaultTimeout(timeout: number | undefined): void {
    this._timeoutSettings.setDefaultTimeout(timeout)
  }

  setDefaultNavigationTimeout(timeout: number | undefined): void {
    this._timeoutSettings.setDefaultNavigationTimeout(timeout)
  }

  /** Playwright's `ChannelOwner.toJSON`: the context's type and guid only. */
  toJSON(): { _type: 'BrowserContext'; _guid: string } {
    return { _type: 'BrowserContext', _guid: this.#guid }
  }

  /** `createRun`'s: registers the run's page before the script starts, so the first `pages()`
   * returns it instead of a second object for the same tab. */
  _addRunPage(guid: string, mainFrameGuid: string, url: string): Page {
    const page = new Page(guid, mainFrameGuid, url, this, this.#connection)
    this.#pages.set(guid, page)
    return page
  }

  /** The `Page` for a record Rust handed out: the object this guid already has, else one
   * built from the record. */
  _pageFrom(created: CreatedPage): Page {
    return this.#pages.get(created.guid) ?? this.#add(created)
  }

  /** The `Page` for a guid Rust just handed out: the object a `pages()` read made while the
   * call was in flight, else one built from the page's creation record. */
  #pageFor(guid: string): Page {
    return this.#pages.get(guid) ?? this.#add(get(guid, 'initializer'))
  }

  #add(created: CreatedPage): Page {
    const page = new Page(
      created.guid,
      created.mainFrame.guid,
      created.mainFrame.initializer.url,
      this,
      this.#connection,
    )
    this.#pages.set(created.guid, page)
    return page
  }
}

/** Playwright's `console` event params (`spec/playwright/browserContext.yml`). */
type ConsoleEventParams = ConsoleMessagePayload & {
  page?: PageChannel
  worker?: unknown
}

/** Playwright's `pageError` event params (`spec/playwright/browserContext.yml`). */
interface PageErrorEventParams {
  error: SerializedError
  page: PageChannel
  location: WebErrorLocation
}

/** Playwright's `request` event params (`spec/playwright/browserContext.yml`), with Neo's full
 * record in place of a `__create__` reference. */
/** The context's `route` event params from Rust (`routes.rs`), naming the request's page. */
interface RouteEventParams {
  route: { guid: string }
  request: Created<RequestInitializer>
  page: { guid: string }
}

interface RequestEventParams {
  request: Created<RequestInitializer>
  page: PageChannel
}

/** Playwright's `response` event params, the response as its full record, with Neo's record of
 * its request, so the event builds both without asking Rust. */
interface ResponseEventParams {
  response: Created<ResponseInitializer>
  request: Created<RequestInitializer>
  page: PageChannel
}

/** Playwright's `requestFinished` and `requestFailed` event params, as Neo sends them. */
interface RequestEndEventParams {
  request: Created<RequestInitializer>
  response?: Created<ResponseInitializer> | null
  failureText?: string
  responseEndTiming: number
  page: PageChannel
}

/** The page guid (`page@T`) a call's guid belongs to: its own, its frames' (`frame@T`,
 * `frame@T/F`), and its requests' and responses' (`request@T:…`). */
function pageGuidOf(guid: string): string {
  const target = guid.slice(guid.indexOf('@') + 1).split(/[/:]/)[0]
  return `page@${target}`
}
