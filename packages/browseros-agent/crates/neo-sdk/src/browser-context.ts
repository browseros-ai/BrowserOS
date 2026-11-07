/**
 * Playwright's `BrowserContext` (`client/browserContext.ts` at 26137face), cut to what a run
 * needs: its pages, `newPage`, the default timeout its pages fall back to, and the events Rust
 * sends on it (`console`, `pageError`, `dialog`, `dialogClosed`), which it fans out to the page
 * each names and to its own listeners in Playwright's order. Scripts receive it as `context`.
 *
 * Its listeners cover the pages the run has a session on: Neo does not attach to the user's
 * other tabs just to listen (med-pw-net design, Decisions).
 *
 * Playwright keeps its page list current from the server's `page` and `close` events. Neo has
 * no events, so `pages()` reads the context's open tabs live, and this class keeps one `Page`
 * per guid across reads. That map is what makes a page the same object every time it comes
 * back (`===`, `includes`, `indexOf`), and what binds each `Page` to its own tab: a `Page` and
 * its main frame are built once, from the record Rust minted their guids in.
 */

import type {
  BrowserContextCloseParams,
  DialogInitializer,
  PageChannel,
  SerializedError,
} from '@browseros/neo-runtime-spec/serve'
import type { Browser } from './browser'
import type { Connection } from './connection'
import { ConsoleMessage, type ConsoleMessagePayload } from './console-message'
import { Dialog } from './dialog'
import { parseSerializedError, targetClosedError } from './errors'
import { ChannelOwner, type EventPipe, type EventTable } from './events'
import { Page } from './page'
import { pickParams } from './protocol'
import { type CreatedPage, get } from './runtime-bridge'
import { TimeoutSettings } from './timeout-settings'
import {
  refuseSignal,
  Waiter,
  type WaitForEventOptions,
  wrapApiCall,
} from './waiter'
import { WebError, type WebErrorLocation } from './web-error'

/**
 * Neo: the context's listener events, by Playwright's names (`Events.BrowserContext`). `close`
 * never happens: the run's context is the user's browser, which outlives the run.
 */
const CONTEXT_EVENTS: EventTable = {
  receiver: 'BrowserContext',
  served: ['console', 'weberror', 'dialog', 'dialogclosed'],
  quiet: ['close', 'backgroundpage'],
  unserved: [
    'download',
    'frameattached',
    'framedetached',
    'framenavigated',
    'page',
    'pageclose',
    'pageload',
    'serviceworker',
    'request',
    'response',
    'requestfailed',
    'requestfinished',
  ],
}

/** Neo: `BrowserContext.close`'s params (`spec/playwright/browserContext.yml`). */
const BROWSER_CONTEXT_CLOSE_KEYS = ['reason'] as const

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
  /** Playwright's `BrowserContext._closingStatus`. */
  #closingStatus: 'none' | 'closing' | 'closed' = 'none'

  constructor(guid: string, connection: Connection, pipe: EventPipe) {
    super(guid, pipe, CONTEXT_EVENTS)
    this.#guid = guid
    this.#connection = connection
    this._pipe = pipe
  }

  /** Playwright's context channel handlers (`client/browserContext.ts` constructor). */
  _onEvent(method: string, params: unknown): void {
    if (method === 'console') this.#onConsole(params as ConsoleEventParams)
    else if (method === 'pageError')
      this.#onPageError(params as PageErrorEventParams)
    else if (method === 'dialog')
      this.#onDialog((params as { dialog: DialogRecord }).dialog)
    else if (method === 'dialogClosed')
      this.#onDialogClosed((params as { dialog: DialogRecord }).dialog)
  }

  #onConsole(event: ConsoleEventParams): void {
    const page = event.page ? this.#pageFor(event.page.guid) : null
    const consoleMessage = new ConsoleMessage(event, page, null)
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
      refuseSignal(options)
      this._refuseUnserved(event)
      const timeout = this._timeoutSettings.timeout(options)
      const predicate =
        typeof optionsOrPredicate === 'function'
          ? optionsOrPredicate
          : optionsOrPredicate.predicate
      const waiter = Waiter.createForEvent()
      waiter.rejectOnTimeout(
        timeout,
        `Timeout ${timeout}ms exceeded while waiting for event "${event}"`,
      )
      if (event !== 'close')
        waiter.rejectOnEvent(this, 'close', () => targetClosedError())
      const result = await waiter.waitForEvent(this, event, predicate)
      waiter.dispose()
      return result
    })
  }

  /** Every open tab of the context, in the order Rust first saw them, the run page first; a
   * page seen before keeps its object. A live read, like every Neo getter. A closed context
   * (or one whose browser was closed) has none. */
  pages(): Page[] {
    if (this.#closingStatus === 'closed' || this.#connection._disconnected)
      return []
    return get(this.#guid, 'pages').map(
      (created) => this.#pages.get(created.guid) ?? this.#add(created),
    )
  }

  /** Playwright's `BrowserContext.browser`: the run's `Browser`, the BrowserOS it is attached
   * to, for the run's own context and for every context it created. */
  browser(): Browser | null {
    return this._browser
  }

  /** Opens a tab on `about:blank` and resolves with its `Page` once the tab answers that URL.
   * Neo opens it in the background, by design: the active tab stays active until a script
   * calls `bringToFront`. A context `browser.newContext` created opens a tab of its own
   * context, with the context's options emulated before the page resolves. */
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

  /**
   * Playwright's `BrowserContext.close` for a context `browser.newContext` created: its pages
   * close, then the context; closing it again does nothing. The run's own context is the user's
   * browser and refuses.
   */
  async close(options: { reason?: string } = {}): Promise<void> {
    if (this.#closingStatus !== 'none') return
    this.#closingStatus = 'closing'
    const open = this._openPages()
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
      this.#closingStatus = 'none'
      throw error
    }
    for (const page of open) page._onClose()
    this._onClose()
  }

  /** Playwright's `BrowserContext._onClose`: the context reads closed, leaves its browser's
   * list, and fires `close`. */
  _onClose(): void {
    if (this.#closingStatus === 'closed') return
    this.#closingStatus = 'closed'
    this._browser?._contexts.delete(this)
    this.emit('close', this)
  }

  /** Neo: the pages of this context that are open now, read before a close that ends them. */
  _openPages(): Page[] {
    if (this.#connection._disconnected) return []
    return [...this.#pages.values()].filter((page) => !page.isClosed())
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

/** Playwright's `pageError` event params (`spec/playwright/browserContext.yml`). */
interface PageErrorEventParams {
  error: SerializedError
  page: PageChannel
  location: WebErrorLocation
}
