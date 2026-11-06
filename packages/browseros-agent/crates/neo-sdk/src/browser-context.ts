/**
 * Playwright's `BrowserContext` (`client/browserContext.ts` at 26137face), cut to what a run
 * needs: its pages, `newPage`, and the default timeout its pages fall back to. Scripts receive
 * it as `context`.
 *
 * Playwright keeps its page list current from the server's `page` and `close` events. Neo has
 * no events, so `pages()` reads the context's open tabs live, and this class keeps one `Page`
 * per guid across reads. That map is what makes a page the same object every time it comes
 * back (`===`, `includes`, `indexOf`), and what binds each `Page` to its own tab: a `Page` and
 * its main frame are built once, from the record Rust minted their guids in.
 */

import type { Connection } from './connection'
import { Page } from './page'
import { type CreatedPage, get } from './runtime-bridge'
import { TimeoutSettings } from './timeout-settings'

export class BrowserContext {
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
  readonly _timeoutSettings = new TimeoutSettings()

  constructor(guid: string, connection: Connection) {
    this.#guid = guid
    this.#connection = connection
  }

  /** Every open tab of the context, in the order Rust first saw them, the run page first; a
   * page seen before keeps its object. A live read, like every Neo getter. */
  pages(): Page[] {
    return get(this.#guid, 'pages').map(
      (created) => this.#pages.get(created.guid) ?? this.#add(created),
    )
  }

  /** Opens a tab on `about:blank` and resolves with its `Page` once the tab answers that URL.
   * Neo opens it in the background, by design: the active tab stays active until a script
   * calls `bringToFront`. */
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

  /** `createRun`'s: registers the run's page before the script starts, so the first `pages()`
   * returns it instead of a second object for the same tab. */
  _addRunPage(guid: string, mainFrameGuid: string, url: string): Page {
    const page = new Page(guid, mainFrameGuid, url, this, this.#connection)
    this.#pages.set(guid, page)
    return page
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
