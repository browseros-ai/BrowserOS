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
import {
  type Observation,
  type PageFact,
  type Reply,
  type RunInit,
  startTimer,
} from './bridge'
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
 * The op timeout of Neo's frame-tree read of a page other than the run's own. Chromium answers
 * no command for a tab whose navigation is pending, and a tab another run is navigating can
 * stall the read's attach, which the native read's own budget (`frame_tree.rs`
 * `SNAPSHOT_BUDGET`, also one second) does not cover.
 */
const LISTED_FRAMES_TIMEOUT_MS = 1_000

/**
 * How long a run's start waits for the first frame-tree reads of the listed tabs the script has
 * not used: one window for all of them, read in parallel, which ends the moment every one has
 * answered. It keeps the observer contract: `frames()` of a listed tab is whole from the first
 * statement when the tab answers, as a host's observer reads another tab (`context.pages()`, then
 * `frames()`, accept_frames.py's `observer`). An answering tab needs milliseconds, its one retry
 * for a frame whose session was still attaching about 100 ms more. What it costs: a tab whose
 * renderer is busy answers nothing until the native read gives up after a second, so it costs a
 * run at most this, once, at the start; no later statement waits for it.
 */
const LISTED_START_GRACE_MS = 300

/** Resolves once `reads` settle or `LISTED_START_GRACE_MS` passed, whichever comes first. */
function withinStartGrace(reads: Promise<unknown>): Promise<void> {
  return new Promise((resolve) => {
    const cancel = startTimer(LISTED_START_GRACE_MS, resolve)
    const done = () => {
      cancel()
      resolve()
    }
    void reads.then(done, done)
  })
}

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
  /**
   * Set once the run's page exists: a page first seen after it is new in the run
   * (`Page._newInRun`).
   */
  #started = false
  /** The frame versions of the reply `_applyReply` is applying, for the pages it makes. */
  #replyVersions: Map<number, number> | null = null

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
      page._newInRun = this.#started
      this.#byTab.set(fact.tab, page)
      // The version the reply that names it gave, taken before its first read, so that read
      // counts as current and the next reply does not read it again.
      const version = this.#replyVersions?.get(fact.tab)
      if (version !== undefined) page._frameVersion(version)
    }
    page._applyFact(fact, at, approved)
    // A page first seen once the run started (a popup, a new page, an op's result, a tab
    // another run opened, one a start read's reply listed) has its tree read now. Such a page
    // holds the script until it leaves a read unanswered (`Page._newInRun`), so the next settle
    // step joins this read and its tree is there by the script's next await. A sync getter
    // cannot wait for it, so in the instant it is handed out (a popup listener) `frames()` lists
    // what was read so far, as upstream's does as a popup opens.
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
    // The versions the start's reads are read at, so the first reply does not take them for
    // changes and read every tab again.
    for (const { tab, version } of init.frames ?? [])
      this.#byTab.get(tab)?._frameVersion(version)
    const page = init.page
      ? this._pageFor(init.page, init.observed)
      : new Page(this, null)
    if (!init.page) this.#unopened = page
    page._working = true
    this.#started = true
    return page
  }

  // Who reads a page's frame tree, and who waits for it. Playwright's `frames()` is synchronous,
  // so the prelude keeps each page's tree and refreshes it with Neo's own `page.frames` read
  // (`Page._refreshFrames`) at three points:
  //
  //   the run's start       every known page               _readInitialFrames
  //   a script op settles   every page whose tree is stale  _settleFrames (bridge.ts onSettle)
  //   waitForTimeout ends   every page the script used     _refreshUsedFrames
  //
  // The script waits only for the pages it used in this run, and for a page the run first saw
  // after it started until that page leaves a read unanswered (`Page._used`). Every other page is
  // read in the background, past the start's short grace for listed tabs
  // (`LISTED_START_GRACE_MS`), and one that did not answer is asked again only once its frame
  // version moves (`Page._askIfChanged`). A page whose renderer is busy answers no read until the
  // native read gives up after a second; when the script waited on every page, each statement
  // paid that second for a tab the script never touched (four seconds for one `ariaSnapshot()`
  // beside one busy tab).

  /**
   * Every known page's frame tree, read in parallel at the run's start (FR-1): the run's page
   * and every tab the init listed. Page facts carry no frame tree, so this is how a tab the
   * script reaches through `pages()` lists its iframes from the first statement. The start waits
   * for the pages the script used, which here is the run's page alone, and for every other listed
   * tab within `LISTED_START_GRACE_MS`: one that answers has its tree by the first statement, and
   * one that does not goes on in the background, bounded by `LISTED_FRAMES_TIMEOUT_MS`. These
   * reads are Neo's own (`Page._refreshFrames`) and move nothing the run's acts move.
   */
  async _readInitialFrames(runPage: Page): Promise<void> {
    const held: Promise<void>[] = []
    const listed: Promise<void>[] = []
    for (const page of new Set([runPage, ...this._knownPages()])) {
      const read = page._readInitialFrames(
        page === runPage ? 0 : LISTED_FRAMES_TIMEOUT_MS,
      )
      if (page._used()) held.push(read)
      else listed.push(read)
    }
    if (listed.length > 0) held.push(withinStartGrace(Promise.all(listed)))
    await Promise.all(held)
  }

  /**
   * Before one of the script's operations settles (bridge.ts `onSettle`): the tree of every page
   * whose frames changed since it was read (every reply carries each tab's version), whose last
   * read was not whole, or whose first read is still in flight. The operation waits for each
   * such page the script used (`Page._settle`), so `frames()` is that page's real tree after
   * every awaited operation. Any other such page is asked again in the background, and only if
   * its frames changed since it was last asked. The bound: one read per such tab per operation,
   * each within `LISTED_FRAMES_TIMEOUT_MS`. These reads are Neo's own, like the start's.
   */
  async _settleFrames(): Promise<void> {
    const held: Promise<void>[] = []
    for (const page of this._knownPages()) {
      if (!page._framesStale()) continue
      if (page._used()) held.push(page._settle(LISTED_FRAMES_TIMEOUT_MS))
      else page._askIfChanged(LISTED_FRAMES_TIMEOUT_MS)
    }
    if (held.length > 0) await Promise.all(held)
  }

  /**
   * One `page.frames` read per open page that has a tab and that the script used (`Page._used`),
   * in parallel, each failure swallowed, as `waitForTimeout` ends (FR-1). A page with no tab yet
   * is skipped, so a wait never opens the run's lazy tab, and a refused read leaves that page's
   * tree as it was. So is a page with a navigation op in flight, which Chromium would not answer
   * until that navigation ends (`Page._navigationsInFlight`). A page the script never acted on is
   * read within `LISTED_FRAMES_TIMEOUT_MS`, as in a settle step: its tab's attach can stall
   * (another run navigating it), and the script's timer must not wait on it.
   */
  async _refreshUsedFrames(api: string): Promise<void> {
    const used = this._knownPages().filter(
      (page) =>
        page._used() && page._tab !== null && page._navigationsInFlight === 0,
    )
    await Promise.all(
      used.map((page) =>
        page
          ._refreshFrames(api, page._working ? 0 : LISTED_FRAMES_TIMEOUT_MS)
          .catch(() => undefined),
      ),
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
    frames,
  }: Reply): void {
    const versions = new Map<number, number>()
    for (const { tab, version } of frames ?? []) versions.set(tab, version)
    for (const [tab, version] of versions)
      this.#byTab.get(tab)?._frameVersion(version)
    const page = this.#unopened
    const held = pickedPage ? this.#byTab.get(pickedPage.tab) : undefined
    const bind =
      defaultPage ?? (held && this.#handed.has(held) ? null : pickedPage)
    if (bind && page) {
      const known = this.#byTab.get(bind.tab)
      this.#unopened = null
      page._tab = bind.tab
      this.#byTab.set(bind.tab, page)
      const version = versions.get(bind.tab)
      if (version !== undefined) page._frameVersion(version)
      // The built-in `page` takes over a tab that already has a document: one the context
      // knew (its object's tree stays with that object) or a task's tab it was picked for. It
      // has read no tree of it, so it reads it now, and the operation that bound it settles
      // after that read (the settle steps join a first read). A tab the door opened for it
      // is new: its reply's version says when its tree is worth reading.
      if (known || bind === pickedPage)
        void page._readInitialFrames(LISTED_FRAMES_TIMEOUT_MS)
    }
    const approved = new Map<number, number>()
    for (const close of observed?.closes ?? [])
      approved.set(close.tab, Math.max(approved.get(close.tab) ?? 0, close.seq))
    this.#replyVersions = versions
    try {
      for (const fact of pages)
        this._pageFor(fact, observed, approved.get(fact.tab) ?? null)
    } finally {
      this.#replyVersions = null
    }
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
