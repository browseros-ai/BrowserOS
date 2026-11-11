/**
 * The one browser context a run sees: every tab of the owner's profile, which Neo never closes
 * or recreates. It owns the run's Page objects (one per tab, so identity holds across
 * popups, facts and handles), the context-level timeout defaults and the run's event hub.
 *
 * `context.pages()` is synchronous, so it lists what the prelude knows: the tabs the run's init
 * listed, the run's page, pages the host's facts named, pages this run opened, and pages events
 * reported.
 */

import { type BindingCallback, exposeOnContext } from './bindings'
import { invoke, type Reply, type RunInit } from './bridge'
import type { Browser } from './browser'
import { EventEmitter } from './emitter'
import { internalError } from './errors'
import { checkReported, EventHub, isReported } from './events'
import { addInitScript, type Disposable } from './init-script'
import { LocatorHandlers } from './locator-handlers'
import { markHandle } from './options'
import { Page } from './page'
import { TimeoutSettings } from './timeouts'
import type { Observation, PageFact } from './wire'

/** The page an op result names: a revived Page, or a raw `{"$neo":"page","tab":1}` brand. */
function pageFact(value: unknown): PageFact | null {
  if (value instanceof Page)
    return value._tab === null ? null : { tab: value._tab }
  if (typeof value !== 'object' || value === null) return null
  const tab: unknown = Reflect.get(value, 'tab')
  return typeof tab === 'number' ? { tab } : null
}

export class BrowserContext extends EventEmitter {
  readonly _timeoutSettings = new TimeoutSettings()
  readonly _events: EventHub
  /** The bindings the context exposed, which a page's calls fall back to. */
  readonly _exposed = new Map<string, BindingCallback>()
  /** The run's locator handlers, on every page: one pull serves them all. */
  readonly _locatorHandlers = new LocatorHandlers()
  _browser: Browser | null = null
  readonly #byTab = new Map<number, Page>()
  /** The run's `page` while it has no tab; the reply that names its tab clears it. */
  #unopened: Page | null = null
  /** Set once the run's page exists: a page first seen after it is working (`Page._working`). */
  #started = false

  constructor() {
    super()
    this._events = new EventHub(this)
    markHandle(this)
  }

  pages(): Page[] {
    return [...this.#byTab.values()].filter((page) => !page.isClosed())
  }

  browser(): Browser | null {
    return this._browser
  }

  isClosed(): boolean {
    return false
  }

  setDefaultTimeout(timeout: number): void {
    this._timeoutSettings.setDefaultTimeout(timeout)
  }

  setDefaultNavigationTimeout(timeout: number): void {
    this._timeoutSettings.setDefaultNavigationTimeout(timeout)
  }

  /** Extension background pages; Neo's context reports none, as Playwright has since 1.56. */
  backgroundPages(): never[] {
    return []
  }

  serviceWorkers(): never[] {
    return []
  }

  /**
   * upstream's `browserContext.addInitScript`. Neo's context is the owner's whole profile, so
   * the script reaches each tab the run writes to, before that write: every document the run
   * loads, in any tab, runs it until the run ends or the disposable is disposed. A popup's
   * first document, which loads before the run acts on the popup, does not.
   */
  addInitScript(
    script: unknown,
    arg?: unknown,
    options?: unknown,
  ): Promise<Disposable> {
    const api = 'browserContext.addInitScript'
    return addInitScript(
      api,
      {
        register: async (source) =>
          (
            await invoke(
              'context.addInitScript',
              { api, timeoutMs: 0 },
              { script: source },
            )
          ).id,
        expose: (name, callback) =>
          exposeOnContext(this, api, name, callback, true),
      },
      script,
      arg,
      options,
    )
  }

  /**
   * upstream's `browserContext.exposeBinding`, reaching each tab as `addInitScript` does: a tab
   * gets the binding at the run's first write to it, then every document of it has
   * `window[name]`.
   */
  exposeBinding(name: string, callback: BindingCallback): Promise<Disposable> {
    return exposeOnContext(this, 'browserContext.exposeBinding', name, callback)
  }

  /** upstream's `browserContext.exposeFunction`. */
  exposeFunction(
    name: string,
    callback: (...args: unknown[]) => unknown,
  ): Promise<Disposable> {
    return exposeOnContext(
      this,
      'browserContext.exposeFunction',
      name,
      (_source, ...args) => Reflect.apply(callback, undefined, args),
    )
  }

  async newPage(): Promise<Page> {
    return await this.#openPage('browserContext.newPage')
  }

  waitForEvent(
    event: string,
    optionsOrPredicate: unknown = {},
  ): Promise<unknown> {
    const since = Date.now()
    return this._events.wait(
      'browserContext.waitForEvent',
      null,
      event,
      optionsOrPredicate,
      since,
      this._timeoutSettings,
    )
  }

  toJSON(): { pages: number } {
    return { pages: this.pages().length }
  }

  protected override _checkEvent(event: string | symbol): void {
    checkReported('browserContext.on', event, 'context')
  }

  protected override _listenersChanged(
    event: string | symbol,
    listening: boolean,
  ): void {
    if (isReported(event, 'context'))
      this._events.listen(null, event, listening)
  }

  /** The page with this tab, created on first sight and refreshed from the fact. */
  _pageFor(
    fact: PageFact,
    at: Observation | null = null,
    approved: number | null = null,
  ): Page {
    let page = this.#byTab.get(fact.tab)
    if (!page) {
      page = new Page(this, fact.tab, fact.url)
      page._working = this.#started
      this.#byTab.set(fact.tab, page)
    }
    page._applyFact(fact, at, approved)
    return page
  }

  /** Results and historical events contribute identity and initial metadata. */
  _pageFromResult(value: unknown, url?: string): Page | null {
    const fact = pageFact(value)
    if (!fact) return null
    if (url !== undefined) fact.url = url
    return this._pageFor(fact)
  }

  /**
   * The agent's `page` parameter: the run's tab when it starts with one, else a page with no
   * tab yet. claw-sdk's door opens that tab for the first op that names the page. The tabs
   * the init lists come first, in the host's order, so `context.pages()` names every tab
   * before any op, and the run's page is the same object as its tab's entry.
   */
  _runPage(init: RunInit): Page {
    for (const fact of init.pages) this._pageFor(fact, init.observed)
    const page = init.page
      ? this._pageFor(init.page, init.observed)
      : new Page(this, null)
    if (!init.page) this.#unopened = page
    page._working = true
    this.#started = true
    return page
  }

  /**
   * One `page.frames` read per open working page that has a tab, in parallel, each failure
   * swallowed, as `waitForTimeout` ends (FR-1). A page with no tab yet is skipped, so a wait
   * never opens the run's lazy tab, and a refused read leaves that page's tree as it was. So is
   * a page with a navigation op in flight, which Chromium would not answer until that
   * navigation ends (`Page._navigationsInFlight`).
   */
  async _refreshWorkingFrames(api: string): Promise<void> {
    const working = this.pages().filter(
      (page) =>
        page._working && page._tab !== null && page._navigationsInFlight === 0,
    )
    await Promise.all(
      working.map((page) => page._refreshFrames(api).catch(() => undefined)),
    )
  }

  /**
   * One reply, before its value is revived. The run's page takes its tab first, so the facts
   * and a result that name that tab find the script's `page` object instead of making a
   * second one.
   */
  _settleWorkerFacts(at: Observation | null): void {
    for (const page of this.#byTab.values()) page._settleWorkerFacts(at)
  }

  _applyReply({ pages, defaultPage, observed }: Reply): void {
    const page = this.#unopened
    if (defaultPage && page) {
      this.#unopened = null
      page._tab = defaultPage.tab
      this.#byTab.set(defaultPage.tab, page)
    }
    const approved = new Map<number, number>()
    for (const close of observed?.closes ?? [])
      approved.set(close.tab, Math.max(approved.get(close.tab) ?? 0, close.seq))
    for (const fact of pages)
      this._pageFor(fact, observed, approved.get(fact.tab) ?? null)
  }

  async #openPage(api: string): Promise<Page> {
    const opened = await invoke('context.newPage', { api, timeoutMs: 0 }, {})
    const page = this._pageFromResult(opened.page, opened.url)
    if (!page) throw internalError(api, 'context.newPage returned no page')
    return page
  }
}
