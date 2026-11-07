/**
 * Playwright's `BrowserContext` (`client/browserContext.ts` at 26137face), cut to what a run
 * needs: its pages, `newPage`, the default timeout its pages fall back to, and the events Rust
 * sends on it (`page`, `console`, `pageError`, `dialog`, `dialogClosed`, `request`, `response`,
 * `requestFinished`, `requestFailed`, `download`), which it fans out to the page each names and
 * to its own listeners in Playwright's order. Its `pageload`, `pageclose` and frame events come
 * from its pages. Scripts receive it as `context`.
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
  BrowserContextSetStorageStateParams,
  BrowserContextStorageStateParams,
  BrowserContextStorageStateResult,
  DialogInitializer,
  HttpCredentials,
  NetworkCookie,
  PageChannel,
  RequestInitializer,
  ResponseInitializer,
  SerializedError,
  SetNetworkCookie,
} from '@browseros/neo-runtime-spec/serve'
import {
  BindingCall,
  type BindingCallback,
  type BindingCallRecord,
} from './binding-call'
import type { Browser } from './browser'
import type { ApiName, Connection } from './connection'
import { ConsoleMessage, type ConsoleMessagePayload } from './console-message'
import { Debugger } from './debugger'
import { Dialog } from './dialog'
import { DisposableStub } from './disposable'
import { Download } from './download'
import { parseSerializedError, targetClosedError } from './errors'
import {
  ChannelOwner,
  type EventPipe,
  type EventTable,
  reportUncaught,
} from './events'
import { type Headers, headersObjectToArray, validateHeaders } from './headers'
import { Disposable, evaluationScript, type InitScript } from './init-script'
import { assertEvaluateOptions, type EvaluateOptions } from './js-handle'
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
 * Neo: the context's listener events, by Playwright's names (`Events.BrowserContext`). The SDK
 * emits `close` itself, when the context closes.
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
    'download',
  ],
  local: ['close'],
  quiet: ['backgroundpage'],
  unserved: ['serviceworker'],
}

/** Neo: why `storageState({ path })` throws: the run has no artifact directory to write it in. */
const STORAGE_STATE_PATH =
  'path: this run has no artifact directory, so Neo cannot write the storage state; the host sets one per run'

/** Neo: why `setStorageState(path)` throws: the run has no artifact directory to read it from. */
const STATE_FILE_PATH =
  'storageState: this run has no artifact directory, so Neo cannot read a state file; the host sets one per run'

/** Neo: why `clearCookies()` without a filter throws. */
const CLEAR_ALL_COOKIES_REFUSED =
  "Neo does not clear every cookie of the user's browser, which would sign the user out of every site: pass clearCookies a name, domain or path"

/** Playwright's `clearCookies` options (`client/network.ts` `ClearNetworkCookieOptions`). */
export interface ClearCookieOptions {
  name?: string | RegExp
  domain?: string | RegExp
  path?: string | RegExp
}

/** Playwright: `client/network.ts` `toClearCookiesParams`: `name: isString(options.name) ? options.name : undefined,`
 * The filters Playwright's client sends: a string or a `RegExp` each, anything else dropped. */
function clearCookieFilter(options: ClearCookieOptions): ClearCookieOptions {
  const kept = (value: unknown) =>
    typeof value === 'string' || value instanceof RegExp ? value : undefined
  return {
    name: kept(options.name),
    domain: kept(options.domain),
    path: kept(options.path),
  }
}

/** Playwright: `server/network.ts` `hasClearCookiesFilter`: `return options.name !== undefined || options.domain !== undefined || options.path !== undefined;` */
function hasClearCookieFilter(filter: ClearCookieOptions): boolean {
  return (
    filter.name !== undefined ||
    filter.domain !== undefined ||
    filter.path !== undefined
  )
}

/** Playwright: `server/network.ts` `cookieMatchesClearFilter`: `return matches('name', options.name) && matches('domain', options.domain) && matches('path', options.path);` */
function clearCookieMatches(
  cookie: NetworkCookie,
  filter: ClearCookieOptions,
): boolean {
  const matches = (
    property: 'name' | 'domain' | 'path',
    value: string | RegExp | undefined,
  ) => {
    if (value === undefined) return true
    if (value instanceof RegExp) {
      value.lastIndex = 0
      return value.test(cookie[property])
    }
    return cookie[property] === value
  }
  return (
    matches('name', filter.name) &&
    matches('domain', filter.domain) &&
    matches('path', filter.path)
  )
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
  /** Whether the client listens to the context's `route` channel event (`_subscribeChannelEvent`). */
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
  /** The context's exposed callbacks, by name (Playwright's `BrowserContext._bindings`). */
  readonly _bindings = new Map<string, BindingCallback>()
  /** The run's event pipe, which this context's pages share. */
  readonly _pipe: EventPipe
  /** Playwright's `BrowserContext._browser`: the run's `Browser`, set when it registers this
   * context (`Browser._didCreateContext`). */
  _browser: Browser | null = null
  /** Playwright's `BrowserContext._ownerPage`: the page `browser.newPage` made this context for. */
  _ownerPage: Page | undefined
  /** Playwright's `_closingStatus`: `closing` from `close()`'s call, `closed` once done. */
  #closingStatus: 'none' | 'closing' | 'closed' = 'none'
  #closeReason: string | undefined
  #debugger: Debugger | undefined

  constructor(guid: string, connection: Connection, pipe: EventPipe) {
    super(guid, pipe, CONTEXT_EVENTS)
    this.#guid = guid
    this.#connection = connection
    this._pipe = pipe
  }

  /** Playwright's `BrowserContext.isClosed`: true from the moment `close()` is called. */
  isClosed(): boolean {
    return this.#closingStatus !== 'none'
  }

  /** Playwright's `_effectiveCloseReason`: the reason `close()` was given, else the one its
   * browser's `close()` was. */
  _closeReason(): string | undefined {
    return this.#closeReason ?? this._browser?._closeReason
  }

  /**
   * Playwright's `BrowserContext.close`. A second call resolves at once; a close that failed
   * leaves the context open, to close again.
   *
   * The run's own context is a connectOverCDP default context, whose close closes its browser
   * (`CRBrowserContext.doClose`), and that close only disconnects: no tab closes (measured on
   * BrowserOS neo with pinned Playwright: every tab stays open, the one its own client opened
   * too). So it is the browser's disconnection, as `browser.close()` is (`Browser._disconnect`):
   * Rust lets go of the run's listeners, stops its other calls and disposes the contexts
   * `browser.newContext` created; every page of the run closes as the script sees it
   * (its `close`, then its context's `pageclose`), then each created context and this one fire
   * `close`, and the browser reads disconnected and fires `disconnected`. From then on `pages()`
   * is empty, every page reads closed, calls fail as closed, and the waits and calls the close
   * interrupted reject with `reason` when given; Rust stops those calls before the close
   * resolves, so none writes input after it.
   *
   * A context `browser.newContext` created closes with its pages, which leave the browser; the
   * run and its other contexts stay.
   */
  async close(options: { reason?: string } = {}): Promise<void> {
    if (this.isClosed()) return
    this.#closeReason = options.reason
    this.#closingStatus = 'closing'
    const send = () =>
      this.#connection.sendContext(
        'browserContext.close',
        this.#guid,
        'close',
        pickParams(BROWSER_CONTEXT_CLOSE_KEYS, {
          reason: options.reason,
        }) as BrowserContextCloseParams,
        0,
      )
    try {
      if (this._browser && this._browser._runContext() === this)
        await this._browser._disconnect(send, options.reason)
      else {
        const open = this._openPages()
        await send()
        for (const page of open) page._onClose()
        this._onClose()
      }
    } catch (error) {
      // Never half-closed: a close Rust did not do leaves the context open, to close again.
      this.#closingStatus = 'none'
      this.#closeReason = undefined
      throw error
    }
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
    else if (method === 'bindingCall')
      void this.#onBindingCall(
        (params as { binding: BindingCallRecord }).binding,
      )
    else if (method === 'download')
      this.#onDownload(params as DownloadEventParams)
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
    this.#assertOwnRouting('route')
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
    this.#assertOwnRouting('unrouteAll')
    await this.#unrouteInternal(this._routes, [], options?.behavior)
  }

  /** Playwright's `BrowserContext.unroute`. */
  async unroute(
    url: RouteURLMatch,
    handler?: RouteHandlerCallback,
  ): Promise<void> {
    this.#assertOwnRouting('unroute')
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

  /** Deviation: `client/browserContext.ts` `BrowserContext.routeFromHAR` Neo does not serve HAR routing yet. */
  async routeFromHAR(_har: string, _options?: unknown): Promise<never> {
    this.#assertOwnRouting('routeFromHAR')
    throw new Error(
      'browserContext.routeFromHAR: neo-runtime: routeFromHAR is not served yet',
    )
  }

  /** Deviation: `client/browserContext.ts` `BrowserContext.routeWebSocket` Neo does not serve WebSocket routing yet. */
  async routeWebSocket(_url: RouteURLMatch, _handler: unknown): Promise<never> {
    this.#assertOwnRouting('routeWebSocket')
    throw new Error(
      'browserContext.routeWebSocket: neo-runtime: routeWebSocket is not served yet',
    )
  }

  /** Deviation: `client/browserContext.ts` `BrowserContext.route` Created-context route, unroute, unrouteAll, routeFromHAR and routeWebSocket calls fail before changing handlers; Neo serves context routes only on the run's own context. */
  #assertOwnRouting(method: string): void {
    if (this.#guid !== 'context@default') {
      throw new Error(
        `browserContext.${method}: neo-runtime: ${method} is not served yet for a context browser.newContext created, only for the run's own context`,
      )
    }
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
      this._subscribeChannelEvent('route', true)
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
      this._subscribeChannelEvent('route', false)
    }
  }

  /** A call of the context's binding: on its page's frame, answered by the context's callback. */
  async #onBindingCall(record: BindingCallRecord): Promise<void> {
    const page = this.#pageFor(record.page.guid)
    const frame = page._frameForGuid(record.frame.guid)
    await this._onBinding(new BindingCall(this.#connection, record, frame))
  }

  /** Playwright: `client/browserContext.ts` `BrowserContext._onBinding`: `await bindingCall.call(func);` */
  async _onBinding(call: BindingCall): Promise<void> {
    const func = this._bindings.get(call.name)
    if (!func) return
    await call.call(func)
  }

  /** Playwright's `BrowserContext._onRequest`: the context's listeners, then the page's. */
  #onRequest({ request, page }: RequestEventParams): void {
    const pageObject = this.#pageFor(page.guid)
    const requestObject = requestFromRecord(
      this.#connection,
      request,
      pageObject,
    )
    this.#contextOf(pageObject).emit('request', requestObject)
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
    this.#contextOf(pageObject).emit('response', responseObject)
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
    this.#contextOf(pageObject).emit('requestfinished', requestObject)
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
    this.#contextOf(pageObject).emit('requestfailed', requestObject)
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
    const consoleMessage = new ConsoleMessage(
      event,
      page,
      null,
      this.#connection,
    )
    page?.emit('console', consoleMessage)
    this.#contextOf(page).emit('console', consoleMessage)
  }

  #onPageError({ error, page, location }: PageErrorEventParams): void {
    const pageObject = this.#pageFor(page.guid)
    const parsedError = parseSerializedError(error)
    this.#contextOf(pageObject).emit(
      'weberror',
      new WebError(pageObject, parsedError, location),
    )
    if (pageObject) pageObject.emit('pageerror', parsedError)
  }

  /**
   * Playwright: `client/page.ts` `Page.constructor`: `this._browserContext.emit(Events.BrowserContext.Download, download);`
   * One `Download` for the page's `download`, then the context's. Rust sends it on the context,
   * naming the page, since a tab the run has not met yet can begin one too.
   */
  #onDownload(event: DownloadEventParams): void {
    const page = this.#pageFor(event.page.guid)
    const download = new Download(
      page,
      event.url,
      event.suggestedFilename,
      event.artifact,
      this.#connection,
    )
    page.emit('download', download)
    this.#contextOf(page).emit('download', download)
  }

  #onDialog(record: DialogRecord): void {
    const dialogObject = this.#dialogFrom(record)
    let hasListeners = this.#contextOf(dialogObject.page()).emit(
      'dialog',
      dialogObject,
    )
    const page = dialogObject.page()
    if (page) hasListeners = page.emit('dialog', dialogObject) || hasListeners
    // Playwright answers here too: its listener may have gone between its subscription and the
    // dialog (a `once` listener of an earlier dialog, an `off` in flight).
    if (!hasListeners) dialogObject._answerDefault()
  }

  #onDialogClosed(record: DialogRecord): void {
    const dialogObject = this.#dialogFrom(record)
    this.#contextOf(dialogObject.page()).emit('dialogclosed', dialogObject)
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
   * accepts, or rejects at the timeout (the context's default, 30 s unless set) or when the
   * context closes.
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
          targetClosedError(this._closeReason()),
        )
      const result = await waiter.waitForEvent(this, event, predicate)
      waiter.dispose()
      return result
    })
  }

  /**
   * Every open tab of the context, in the order Rust first saw them, the run page first; a
   * page seen before keeps its object. A live read, like every Neo getter, so it can list a new
   * tab before that tab's `page` event has run; after the context closed, nothing.
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

  /** Playwright's `browserContext.exposeFunction`: `window[name](...args)` in every frame of every page of the context calls
   * `callback(...args)` in the run and answers with its result. `dispose()` removes it. */
  async exposeFunction(
    name: string,
    // biome-ignore lint/suspicious/noExplicitAny: Playwright's callback type.
    callback: (...args: any[]) => unknown,
  ): Promise<Disposable> {
    return await this.#expose(
      'browserContext.exposeFunction',
      name,
      (_source, ...args) => callback(...args),
    )
  }

  /** Playwright's `browserContext.exposeBinding`: as `exposeFunction`, with the call's
   * `{ context, page, frame }` first. */
  async exposeBinding(
    name: string,
    callback: BindingCallback,
  ): Promise<Disposable> {
    return await this.#expose('browserContext.exposeBinding', name, callback)
  }

  async #expose(
    apiName: ApiName,
    name: string,
    callback: BindingCallback,
  ): Promise<Disposable> {
    // Deviation: `client/browserContext.ts` `BrowserContext.exposeBinding`: `this._bindings.set(name, callback);` Playwright sets the callback after its answer; Neo before, as the answer's events, a first page call among them, run before the caller goes on.
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

  /** Playwright's `browserContext.addInitScript`: `script` (a function called with `arg`, source text, or
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
      // Deviation: `client/browserContext.ts` `BrowserContext.addInitScript`: `return DisposableObject.from((await this._channel.addInitScript({ source }, kNoTimeout)).disposable);` Playwright sends no signal and ignores one; Neo refuses it.
      throw new Error(
        `browserContext.addInitScript: Neo does not support signal yet`,
      )
    if (options?.exposeFunctions)
      // Deviation: `client/browserContext.ts` `BrowserContext.addInitScript`: `return await addInitScriptWithExposedFunctions(this, script, arg);` Neo has no page-to-realm callback yet.
      throw new Error(
        `browserContext.addInitScript: Neo does not support exposeFunctions yet`,
      )
    const source = evaluationScript('browserContext.addInitScript', script, arg)
    const result = await this.#connection.sendInit(
      'browserContext.addInitScript',
      this.#guid,
      'addInitScript',
      { source },
      0,
    )
    return new Disposable(this.#connection, result.disposable.guid)
  }

  /** Playwright's `BrowserContext.debugger`: the context's own debugger (`debugger.ts`), made
   * when a script first reads it, with the guid Rust names for it. */
  get debugger(): Debugger {
    this.#debugger ??= new Debugger(
      get(this.#guid, 'debugger'),
      this.#connection,
      this._pipe,
    )
    return this.#debugger
  }

  /** Playwright's `BrowserContext.browser`: the run's `Browser`, the BrowserOS it is attached
   * to, for the run's own context and for every context it created. */
  browser(): Browser | null {
    return this._browser
  }

  /**
   * Opens a tab on `about:blank` and resolves with its `Page` once the tab answers that URL; a
   * `page` listener has run for it by then. A context `browser.newContext` created opens a tab
   * of its own context, with the context's options emulated before the page resolves.
   *
   * Deviation: `server/chromium/crBrowser.ts` `CRBrowserContext.doCreateNewPage`: `const { targetId } = await this._browser._session.send('Target.createTarget', { url: 'about:blank', browserContextId: this._browserContextId });`
   * Playwright's new page takes the window's focus. Neo opens it in the background of the run
   * page's window, by design: the user's active tab stays active until a script calls
   * `bringToFront`.
   */
  async newPage(): Promise<Page> {
    // Playwright: `client/browserContext.ts` `BrowserContext.newPage`: `throw new Error('Please use browser.newContext()');`
    if (this._ownerPage) throw new Error('Please use browser.newContext()')
    const { page } = await this.#connection.sendContext(
      'browserContext.newPage',
      this.#guid,
      'newPage',
      {},
      0,
    )
    const created = this.#pageFor(page.guid)
    // Playwright's context emits `page` before `newPage()` resolves. Rust's target discovery
    // emits it for the run's own context; a context the run created has none, so here.
    if (this._browser && this._browser._runContext() !== this)
      this.emit('page', created)
    return created
  }

  /** Playwright: `client/browserContext.ts` `BrowserContext.cookies`: `if (urls && typeof urls === 'string')`
   * Every cookie of the user's browser profile, which is the run's context, or those a request
   * to one of `urls` would send. */
  async cookies(urls?: string | string[]): Promise<NetworkCookie[]> {
    const list = !urls ? [] : typeof urls === 'string' ? [urls] : urls
    const { cookies } = await this.#connection.sendContext(
      'browserContext.cookies',
      this.#guid,
      'cookies',
      { urls: list },
      0,
    )
    return cookies
  }

  /** Playwright: `client/browserContext.ts` `BrowserContext.addCookies`: `await this._channel.addCookies({ cookies }, kNoTimeout);`
   * The cookies stay in the user's profile after the run, as a site's would. */
  async addCookies(cookies: SetNetworkCookie[]): Promise<void> {
    await this.#connection.sendContext(
      'browserContext.addCookies',
      this.#guid,
      'addCookies',
      { cookies },
      0,
    )
  }

  /**
   * Playwright: `server/browserContext.ts` `BrowserContext.clearCookies`: `const cookiesToExpire = currentCookies.filter(cookie => network.cookieMatchesClearFilter(cookie, options));`
   * The cookies matching every filter given expire, the user's others untouched: Playwright's
   * server logic, run here, where a `RegExp` filter is the script's own.
   *
   * Deviation: `server/browserContext.ts` `BrowserContext.clearCookies`: `await this.doClearCookies();`
   * With no filter Playwright clears every cookie of its context. Neo's context is the user's
   * own browser, where that would sign the user out of every site, so Neo refuses the call
   * without a `name`, `domain` or `path`.
   */
  async clearCookies(options: ClearCookieOptions = {}): Promise<void> {
    const filter = clearCookieFilter(options)
    if (!hasClearCookieFilter(filter))
      throw new Error(CLEAR_ALL_COOKIES_REFUSED)
    const { cookies } = await this.#connection.sendContext(
      'browserContext.clearCookies',
      this.#guid,
      'cookies',
      { urls: [] },
      0,
    )
    const expired = cookies
      .filter((cookie) => clearCookieMatches(cookie, filter))
      // Playwright: `server/browserContext.ts` `BrowserContext.clearCookies`: `// Keep the original value: browsers reject or ignore cookies with both empty name and value.`
      .map((cookie) => ({ ...cookie, expires: 0 }))
    if (!expired.length) return
    await this.#connection.sendContext(
      'browserContext.clearCookies',
      this.#guid,
      'addCookies',
      { cookies: expired },
      0,
    )
  }

  /** Playwright: `client/browserContext.ts` `BrowserContext.setExtraHTTPHeaders`: `network.validateHeaders(headers);`
   * Sent with every request of the run's tabs, each page's own over them, until the run ends. */
  async setExtraHTTPHeaders(headers: Headers): Promise<void> {
    validateHeaders(headers)
    await this.#connection.sendContext(
      'browserContext.setExtraHTTPHeaders',
      this.#guid,
      'setExtraHTTPHeaders',
      { headers: headersObjectToArray(headers) },
      0,
    )
  }

  /** Playwright: `client/browserContext.ts` `BrowserContext.setOffline`: `await this._channel.setOffline({ offline }, kNoTimeout);`
   * The run's tabs, until the run ends. */
  async setOffline(offline: boolean): Promise<void> {
    await this.#connection.sendContext(
      'browserContext.setOffline',
      this.#guid,
      'setOffline',
      { offline },
      0,
    )
  }

  /** Playwright: `client/browserContext.ts` `BrowserContext.setGeolocation`: `await this._channel.setGeolocation({ geolocation: geolocation || undefined }, kNoTimeout);`
   * The run's tabs, until the run ends; `null` clears it. */
  async setGeolocation(
    geolocation: {
      longitude: number
      latitude: number
      accuracy?: number
    } | null,
  ): Promise<void> {
    await this.#connection.sendContext(
      'browserContext.setGeolocation',
      this.#guid,
      'setGeolocation',
      geolocation ? { geolocation } : {},
      0,
    )
  }

  /** Playwright: `client/browserContext.ts` `BrowserContext.grantPermissions`: `await this._channel.grantPermissions({ permissions, ...options }, kNoTimeout);`
   * The run's own grant, on `origin` or every origin, over the user's settings until the run
   * clears it or ends (neo-runtime `session_state.rs` `Permissions`). */
  async grantPermissions(
    permissions: string[],
    options?: { origin?: string },
  ): Promise<void> {
    await this.#connection.sendContext(
      'browserContext.grantPermissions',
      this.#guid,
      'grantPermissions',
      { permissions, ...options },
      0,
    )
  }

  /** Playwright: `client/browserContext.ts` `BrowserContext.clearPermissions`: `await this._channel.clearPermissions({}, kNoTimeout);`
   * The run's grants go; another run's, or the user's own settings, stay. */
  async clearPermissions(): Promise<void> {
    await this.#connection.sendContext(
      'browserContext.clearPermissions',
      this.#guid,
      'clearPermissions',
      {},
      0,
    )
  }

  /** Playwright: `client/browserContext.ts` `BrowserContext.setHTTPCredentials`: `await this._channel.setHTTPCredentials({ httpCredentials: toHttpCredentialsProtocol(httpCredentials || undefined) }, kNoTimeout);`
   * Neo refuses it for now (neo-runtime `session_state.rs`): its routes do not yet answer a
   * server's credentials challenge. */
  async setHTTPCredentials(
    httpCredentials: HttpCredentials | HttpCredentials[] | null,
  ): Promise<void> {
    const list = !httpCredentials
      ? []
      : Array.isArray(httpCredentials)
        ? httpCredentials
        : [httpCredentials]
    await this.#connection.sendContext(
      'browserContext.setHTTPCredentials',
      this.#guid,
      'setHTTPCredentials',
      list.length ? { httpCredentials: list } : {},
      0,
    )
  }

  /**
   * Playwright: `client/browserContext.ts` `BrowserContext.storageState`: `const state = await this._channel.storageState({ indexedDB: options.indexedDB, opfs: options.opfs, credentials: options.credentials }, kNoTimeout);`
   * Every cookie of the user's browser, and the `localStorage` of the origins the run's pages
   * visited and still have open (neo-runtime `session_state.rs`).
   *
   * Deviation: `client/browserContext.ts` `BrowserContext.storageState`: `await fs.promises.writeFile(options.path, JSON.stringify(state, undefined, 2), 'utf8');`
   * Playwright writes `path` anywhere on its host. Neo writes a script's files only inside the
   * run's artifact directory, which the host gives the run; without one, `path` throws.
   */
  async storageState(
    options: {
      path?: string
      indexedDB?: boolean
      opfs?: boolean
      credentials?: boolean
    } = {},
  ): Promise<BrowserContextStorageStateResult> {
    if (options.path !== undefined) throw new Error(STORAGE_STATE_PATH)
    const params: BrowserContextStorageStateParams = {}
    if (options.indexedDB !== undefined) params.indexedDB = options.indexedDB
    if (options.opfs !== undefined) params.opfs = options.opfs
    if (options.credentials !== undefined)
      params.credentials = options.credentials
    return await this.#connection.sendContext(
      'browserContext.storageState',
      this.#guid,
      'storageState',
      params,
      0,
    )
  }

  /**
   * Playwright: `client/browserContext.ts` `BrowserContext.setStorageState`: `await this._channel.setStorageState({ storageState: state }, kNoTimeout);`
   * Adds the state's cookies and replaces each listed origin's `localStorage` through the run's
   * open page on it; nothing else of the user's browser is cleared (neo-runtime
   * `session_state.rs`, Deviation).
   *
   * Deviation: `client/browserContext.ts` `prepareStorageState`: `return JSON.parse(await fs.promises.readFile(storageState, 'utf8'));`
   * Playwright reads a state file anywhere on its host. Neo reads a script's files only inside
   * the run's artifact directory, which the host gives the run; without one, a path throws.
   */
  async setStorageState(
    storageState: string | BrowserContextSetStorageStateParams['storageState'],
  ): Promise<void> {
    if (typeof storageState === 'string') throw new Error(STATE_FILE_PATH)
    await this.#connection.sendContext(
      'browserContext.setStorageState',
      this.#guid,
      'setStorageState',
      storageState ? { storageState } : {},
      0,
    )
  }

  setDefaultTimeout(timeout: number | undefined): void {
    this._timeoutSettings.setDefaultTimeout(timeout)
  }

  setDefaultNavigationTimeout(timeout: number | undefined): void {
    this._timeoutSettings.setDefaultNavigationTimeout(timeout)
  }

  /** Playwright's `BrowserContext._onClose`: the context reads closed, leaves its browser's
   * list, and fires `close`. */
  _onClose(): void {
    if (this.#closingStatus === 'closed') return
    this.#closingStatus = 'closed'
    this._browser?._contexts.delete(this)
    this.emit('close', this)
  }

  /**
   * Neo: the pages a close of this context ends, read before it, since a created context's tabs
   * are gone once Rust answers: every tab the context lists and every page the run holds of it
   * that is still open.
   */
  _openPages(): Page[] {
    const pages = this.pages()
    for (const page of this.#pages.values())
      if (!pages.includes(page) && !page.isClosed()) pages.push(page)
    return pages
  }

  /** Neo: this context's `Page` for `guid`, if it was handed one. */
  _knownPage(guid: string): Page | undefined {
    return this.#pages.get(guid)
  }

  /** Playwright's `ChannelOwner.toJSON`: the context's type and guid only. */
  toJSON(): { _type: 'BrowserContext'; _guid: string } {
    return { _type: 'BrowserContext', _guid: this.#guid }
  }

  /** The context a page's context-level events belong to: its own, for a page of a context the
   * run created, whose events reach the run's context (Rust names that one). Every context-level
   * event goes through it, as each Playwright context hears only its own pages. */
  #contextOf(page: Page | null): BrowserContext {
    return page ? page.context() : this
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
   * call was in flight, or another context's (an event of a created context's page), else one
   * built from the page's creation record. */
  #pageFor(guid: string): Page {
    return (
      this.#pages.get(guid) ??
      this._browser?._knownPage(guid) ??
      this.#add(get(guid, 'initializer'))
    )
  }

  #add(created: CreatedPage): Page {
    const page = new Page(
      created.guid,
      created.mainFrame.guid,
      created.mainFrame.initializer.url,
      this,
      this.#connection,
      created.initializer.viewportSize,
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

/** Playwright's `download` event params (`spec/playwright/page.yml`), sent on the context with
 * the page it names, and `artifact` the download's guid. */
interface DownloadEventParams {
  page: PageChannel
  url: string
  suggestedFilename: string
  artifact: string
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
export function pageGuidOf(guid: string): string {
  const target = guid.slice(guid.indexOf('@') + 1).split(/[/:]/)[0]
  return `page@${target}`
}
