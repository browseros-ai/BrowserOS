import { captureSite } from './call-site'
import { send } from './generated'
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
import type { Observation, PageFact, Reply, RunInit } from './bridge'
import type { Browser } from './browser'
import { EventEmitter } from './emitter'
import { internalError } from './errors'
import { checkReported, EventHub, isReported } from './events'
import { addInitScript, type Disposable } from './init-script'
import { LocatorHandlers } from './locator-handlers'
import { markHandle } from './options'
import { Page } from './page'
import { TimeoutSettings } from './timeouts'

/**
 * How long a run's start waits for one listed tab's frame tree. Chromium answers no command
 * for a tab whose navigation is pending, so a tab another run is navigating would otherwise
 * hold this run's first statement; the tab's tree is read again at the script's next wait.
 */
const LISTED_FRAMES_TIMEOUT_MS = 1_000

/**
 * The least time between two of the settle step's reads of one tab (`Page._framesStale`): a page
 * that adds or swaps frames constantly costs at most four tree reads a second, however many
 * operations the script makes on it, and its `frames()` lags the page by at most this much.
 */
const SETTLE_GAP_MS = 250

/** The page an op result names: a revived Page, or a raw `{"$neo":"page","tab":1}` brand. */
function pageFact(value: unknown): PageFact | null {
  if (value instanceof Page)
    return value._tab === null ? null : { tab: value._tab }
  if (typeof value !== 'object' || value === null) return null
  const tab: unknown = Reflect.get(value, 'tab')
  if (typeof tab !== 'number') return null
  const fact: PageFact = { tab }
  const taskId: unknown = Reflect.get(value, 'taskId')
  if (taskId === null || typeof taskId === 'string') fact.taskId = taskId
  return fact
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
  /**
   * The Page objects that reached the script: listed by `pages()`, or named by an op result,
   * an event or an element handle. Binding the run's `page` to a tab replaces that tab's entry,
   * and events, binding calls and worker facts reach the entry alone, so a replaced object the
   * script holds would lose its listeners, exposed bindings and workers. A task pick therefore
   * binds `page` only to a tab whose object is not among these (`_applyReply`).
   */
  readonly #handed = new WeakSet<Page>()
  /** Set once the run's page exists: a page first seen after it is working (`Page._working`). */
  #started = false
  /**
   * The newest frame-change head a reply gave (`Envelope.framesHead`): it moves when any tab's
   * frames change, including a tab whose facts no reply carried (`Page._framesStale`).
   */
  _framesHead = 0

  constructor() {
    super()
    this._events = new EventHub(this)
    markHandle(this)
  }

  pages(): Page[] {
    const pages = this._knownPages()
    for (const page of pages) this.#handed.add(page)
    return pages
  }

  /** `pages()` for the prelude's own reads, which hand no Page to the script. */
  _knownPages(): Page[] {
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
    const site = captureSite()
    const api = 'browserContext.addInitScript'
    return addInitScript(
      api,
      {
        register: async (source) =>
          await send['context.addInitScript'](
            site,
            { api, timeoutMs: 0 },
            source,
          ),
        expose: (name, callback) =>
          exposeOnContext(this, site, api, name, callback, true),
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
    const site = captureSite()
    return exposeOnContext(
      this,
      site,
      'browserContext.exposeBinding',
      name,
      callback,
    )
  }

  /** upstream's `browserContext.exposeFunction`. */
  exposeFunction(
    name: string,
    callback: (...args: unknown[]) => unknown,
  ): Promise<Disposable> {
    const site = captureSite()
    return exposeOnContext(
      this,
      site,
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
    const site = captureSite()
    const since = Date.now()
    return this._events.wait(
      'browserContext.waitForEvent',
      site,
      null,
      event,
      optionsOrPredicate,
      since,
      this._timeoutSettings,
    )
  }

  toJSON(): { pages: number } {
    return { pages: this._knownPages().length }
  }

  protected override _checkEvent(event: string | symbol): void {
    checkReported('browserContext.on', event, 'context')
  }

  protected override _listenersChanged(
    event: string | symbol,
    listening: boolean,
  ): void {
    const site = captureSite()
    if (isReported(event, 'context'))
      this._events.listen(site, null, event, listening)
  }

  /** The page with this tab, created on first sight and refreshed from the fact. */
  _pageFor(
    fact: PageFact,
    at: Observation | null = null,
    approved: number | null = null,
  ): Page {
    let page = this.#byTab.get(fact.tab)
    const fresh = !page
    if (!page) {
      page = new Page(this, fact.tab, fact.url)
      page._working = this.#started
      this.#byTab.set(fact.tab, page)
    }
    page._applyFact(fact, at, approved)
    // A page first seen once the run started (a popup, a new page, an op's result, a tab
    // another run opened, one a start read's reply listed) has its tree read now, in the
    // background, as the start reads every listed tab's: it is there by the script's next
    // await. A sync getter cannot wait for it, so in the instant it is handed out (a popup
    // listener) `frames()` lists what was read so far, as upstream's does as a popup opens.
    if (fresh && this.#started && !page.isClosed())
      void page._readInitialFrames(LISTED_FRAMES_TIMEOUT_MS)
    return page
  }

  /**
   * Results and historical events contribute identity and initial metadata. The Page goes to
   * the script, as a result's value or an event's argument.
   */
  _pageFromResult(value: unknown, url?: string): Page | null {
    const fact = pageFact(value)
    if (!fact) return null
    if (url !== undefined) fact.url = url
    return this._handOut(this._pageFor(fact))
  }

  /** Records that `page` reached the script (`#handed`). */
  _handOut(page: Page): Page {
    this.#handed.add(page)
    return page
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
   * Every page's frame tree, read in parallel before the agent's first statement (FR-1): the
   * run's page and every tab the init listed. Page facts carry no frame tree, so without this a
   * tab the script reaches through `pages()`, another task's say, listed only its main frame. A
   * listed tab's read is bounded (`LISTED_FRAMES_TIMEOUT_MS`): a tab another run is navigating
   * answers no command until that navigation commits, and the script must not wait on it. These
   * reads are Neo's own (`Page._refreshFrames`), so they cost one `page.frames` per open tab
   * and move nothing the run's acts move.
   */
  async _readInitialFrames(runPage: Page): Promise<void> {
    const pages = new Set([runPage, ...this._knownPages()])
    await Promise.all(
      [...pages].map((page) =>
        page._readInitialFrames(
          page === runPage ? 0 : LISTED_FRAMES_TIMEOUT_MS,
        ),
      ),
    )
  }

  /**
   * Before one of the script's operations resolves (bridge.ts `onSettle`): reads again, in
   * parallel, the tree of every page whose frames changed since it was read or whose last read
   * was partial, so `frames()` is each page's real tree after every awaited operation. A page
   * whose frames did not change costs nothing here. These reads are Neo's own, like the start's.
   */
  async _settleFrames(): Promise<void> {
    const now = Date.now()
    const stale = this._knownPages().filter((page) =>
      page._framesStale(now, SETTLE_GAP_MS),
    )
    if (stale.length === 0) return
    await Promise.all(
      stale.map((page) =>
        page
          ._refreshFrames('page.frames', LISTED_FRAMES_TIMEOUT_MS)
          .catch(() => page._framesUnknown()),
      ),
    )
  }

  /**
   * One `page.frames` read per open page that has a tab and that the script acted on
   * (`Page._working`) or whose frames it read (`Page._framesWatched`), in parallel, each failure
   * swallowed, as `waitForTimeout` ends (FR-1). A page with no tab yet is skipped, so a wait
   * never opens the run's lazy tab, and a refused read leaves that page's tree as it was. So is
   * a page with a navigation op in flight, which Chromium would not answer until that
   * navigation ends (`Page._navigationsInFlight`).
   */
  async _refreshWorkingFrames(api: string): Promise<void> {
    const working = this._knownPages().filter(
      (page) =>
        (page._working || page._framesWatched) &&
        page._tab !== null &&
        page._navigationsInFlight === 0,
    )
    await Promise.all(
      working.map((page) => page._refreshFrames(api).catch(() => undefined)),
    )
  }

  _settleWorkerFacts(at: Observation | null): void {
    for (const page of this.#byTab.values()) page._settleWorkerFacts(at)
  }

  /**
   * One reply, before its value is revived. The run's page takes its tab first, so the facts
   * and a result that name that tab find the script's `page` object instead of making a
   * second one. `defaultPage` always binds it: the door ran one of `page`'s own ops on that
   * tab. `pickedPage` (a task pick) binds it only while that tab's object has not reached the
   * script; otherwise `page` stays unopened until its first action, whose `defaultPage` names
   * the same tab, so the object the script holds keeps working until the script uses `page`.
   */
  _applyReply({
    pages,
    defaultPage,
    pickedPage,
    observed,
    framesHead,
  }: Reply): void {
    this._framesHead = Math.max(this._framesHead, framesHead)
    const page = this.#unopened
    const held = pickedPage ? this.#byTab.get(pickedPage.tab) : undefined
    const bind =
      defaultPage ?? (held && this.#handed.has(held) ? null : pickedPage)
    if (bind && page) {
      this.#unopened = null
      page._tab = bind.tab
      this.#byTab.set(bind.tab, page)
    }
    const approved = new Map<number, number>()
    for (const close of observed?.closes ?? [])
      approved.set(close.tab, Math.max(approved.get(close.tab) ?? 0, close.seq))
    for (const fact of pages)
      this._pageFor(fact, observed, approved.get(fact.tab) ?? null)
  }

  async #openPage(api: string): Promise<Page> {
    const site = captureSite()
    const opened = await send['context.newPage'](
      site,
      { api, timeoutMs: 0 },
      {},
    )
    const page = this._pageFromResult(opened.page, opened.url)
    if (!page) throw internalError(api, 'context.newPage returned no page')
    return page
  }
}
