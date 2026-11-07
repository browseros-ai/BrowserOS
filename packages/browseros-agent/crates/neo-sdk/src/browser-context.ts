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
  DialogInitializer,
  PageChannel,
  RequestInitializer,
  ResponseInitializer,
  SerializedError,
} from '@browseros/neo-runtime-spec/serve'
import type { Browser } from './browser'
import type { Connection } from './connection'
import { ConsoleMessage, type ConsoleMessagePayload } from './console-message'
import { Debugger } from './debugger'
import { Dialog } from './dialog'
import { Download } from './download'
import { parseSerializedError, targetClosedError } from './errors'
import { ChannelOwner, type EventPipe, type EventTable } from './events'
import { type Created, requestFromRecord, responseFromRecord } from './network'
import { Page } from './page'
import { BROWSER_CONTEXT_CLOSE_KEYS, pickParams } from './protocol'
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

/** A dialog record Rust sends: Playwright's `{guid, initializer}`. */
interface DialogRecord {
  guid: string
  initializer: DialogInitializer
}

export class BrowserContext extends ChannelOwner {
  /** `context@default`, the guid Rust serves this context's getters and calls on. */
  readonly #guid: string
  readonly #connection: Connection
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
    else if (method === 'download')
      this.#onDownload(params as DownloadEventParams)
    else if (method === 'request') this.#onRequest(params as RequestEventParams)
    else if (method === 'response')
      this.#onResponse(params as ResponseEventParams)
    else if (method === 'requestFinished')
      this.#onRequestFinished(params as RequestEndEventParams)
    else if (method === 'requestFailed')
      this.#onRequestFailed(params as RequestEndEventParams)
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
    this.emit('download', download)
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
    return this.#pageFor(page.guid)
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
   * run created, whose events reach the run's context (Rust names that one). */
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
