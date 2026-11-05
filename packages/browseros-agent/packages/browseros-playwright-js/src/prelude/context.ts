/**
 * The one browser context a run sees: every tab of the owner's profile, which Neo never closes
 * or recreates. It owns the run's Page objects (one per page id, so identity holds across
 * popups, facts and handles), the context-level timeout defaults and the run's event hub.
 *
 * `context.pages()` is synchronous, so it lists what the prelude knows: the tabs the run's init
 * listed, the run's page, pages the host's facts named, pages this run opened, and pages events
 * reported.
 */

import { invoke, type Reply, type RunInit } from './bridge'
import type { Browser } from './browser'
import { EventEmitter } from './emitter'
import { clientError } from './errors'
import { checkReported, EventHub, isReported } from './events'
import { markHandle } from './options'
import { Page } from './page'
import { TimeoutSettings } from './timeouts'
import type { PageFact } from './wire'

/** The page an op result names: a revived Page, or a raw `{"$neo":"page","id":7,"tab":1}` brand. */
function pageFact(value: unknown): PageFact | null {
  if (value instanceof Page)
    return value._id === null
      ? null
      : value._tab === null
        ? { id: value._id }
        : { id: value._id, tab: value._tab }
  if (typeof value !== 'object' || value === null) return null
  const id: unknown = Reflect.get(value, 'id')
  const tab: unknown = Reflect.get(value, 'tab')
  if (typeof id !== 'number') return null
  return typeof tab === 'number' ? { id, tab } : { id }
}

export class BrowserContext extends EventEmitter {
  readonly _timeoutSettings = new TimeoutSettings()
  readonly _events: EventHub
  _browser: Browser | null = null
  readonly #byId = new Map<number, Page>()
  /** The run's `page` while it has no tab; the reply that names its tab clears it. */
  #unopened: Page | null = null

  constructor() {
    super()
    this._events = new EventHub(this)
    markHandle(this)
  }

  pages(): Page[] {
    return [...this.#byId.values()].filter((page) => !page.isClosed())
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

  /** The page with this id, created on first sight and refreshed from the fact. */
  _pageFor(fact: PageFact): Page {
    let page = this.#byId.get(fact.id)
    if (!page) {
      page = new Page(this, fact.id, fact.url)
      this.#byId.set(fact.id, page)
    }
    page._applyFact(fact)
    return page
  }

  /** A page from an op result (`page.opener`, an event), or null for none; `url` refreshes it. */
  _pageFromResult(value: unknown, url?: string): Page | null {
    const fact = pageFact(value)
    if (!fact) return null
    return this._pageFor(url === undefined ? fact : { ...fact, url })
  }

  /**
   * The agent's `page` parameter: the run's tab when it starts with one, else a page with no
   * tab yet. claw-sdk's door opens that tab for the first op that names the page. The tabs
   * the init lists come first, in the host's order, so `context.pages()` names every tab
   * before any op, and the run's page is the same object as its tab's entry.
   */
  _runPage(init: RunInit): Page {
    for (const fact of init.pages) this._pageFor(fact)
    if (init.page) return this._pageFor(init.page)
    this.#unopened = new Page(this, null)
    return this.#unopened
  }

  /**
   * One reply, before its value is revived. The run's page takes its tab first, so the facts
   * and a result that name that tab find the script's `page` object instead of making a
   * second one.
   */
  _applyReply({ pages, defaultPage }: Reply): void {
    const page = this.#unopened
    if (defaultPage && page) {
      this.#unopened = null
      page._id = defaultPage.id
      page._tab = defaultPage.tab
      this.#byId.set(defaultPage.id, page)
    }
    for (const fact of pages) this._pageFor(fact)
  }

  async #openPage(api: string): Promise<Page> {
    const opened = await invoke('context.newPage', { api, timeoutMs: 0 }, {})
    const page = this._pageFromResult(opened.page, opened.url)
    if (!page) throw clientError(api, 'context.newPage returned no page')
    return page
  }
}
