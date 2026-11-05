/**
 * The one browser context a run sees: every tab of the owner's profile, which Neo never closes
 * or recreates. It owns the run's Page objects (one per page id, so identity holds across
 * popups, facts and handles), the context-level timeout defaults and the run's event hub.
 *
 * `context.pages()` is synchronous, so it lists what the prelude knows: the run-start seed,
 * pages the host's facts named, pages this run opened, and pages events reported.
 */

import { invoke, readSeed, type Seed } from './bridge'
import type { Browser } from './browser'
import { EventEmitter } from './emitter'
import { clientError } from './errors'
import { checkReported, EventHub, isReported } from './events'
import { markHandle } from './options'
import { Page } from './page'
import { TimeoutSettings } from './timeouts'
import type { PageEntry, PageFact } from './wire'

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

/** The fact a `context.newPage` entry gives: the page and its URL. */
function entryFact(entry: PageEntry): PageFact | null {
  const fact = pageFact(entry.page)
  return fact ? { ...fact, url: entry.url } : null
}

export class BrowserContext extends EventEmitter {
  readonly _timeoutSettings = new TimeoutSettings()
  readonly _events: EventHub
  _browser: Browser | null = null
  readonly #byId = new Map<number, Page>()
  /** The `page` global: the seeded default tab, or one opened on its first op. */
  readonly _defaultPage: Page

  constructor(seed: Seed = readSeed()) {
    super()
    this._events = new EventHub(this)
    for (const fact of seed.pages) this._pageFor(fact)
    this._defaultPage =
      seed.page === null
        ? new Page(this, null)
        : this._pageFor({ id: seed.page })
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

  _applyFacts(facts: PageFact[]): void {
    for (const fact of facts) this._pageFor(fact)
  }

  /** Opens the tab behind the lazy `page` global, which then keeps its object identity. */
  async _openDefaultPage(api: string, page: Page): Promise<number> {
    const fact = entryFact(
      await invoke('context.newPage', { api, timeoutMs: 0 }, {}),
    )
    if (!fact) throw clientError(api, 'context.newPage returned no page')
    page._id = fact.id
    page._tab = fact.tab ?? null
    this.#byId.set(fact.id, page)
    page._applyFact(fact)
    return fact.id
  }

  async #openPage(api: string): Promise<Page> {
    const opened = await invoke('context.newPage', { api, timeoutMs: 0 }, {})
    const page = this._pageFromResult(opened.page, opened.url)
    if (!page) throw clientError(api, 'context.newPage returned no page')
    return page
  }
}
