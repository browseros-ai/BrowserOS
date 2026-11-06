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
   * By guid, every page the last `pages()` listed, plus those handed out since: the run page,
   * `newPage` results. A page the next `pages()` does not list leaves it, as a closed page
   * leaves Playwright's list. Dropping that rule would keep every tab the run ever saw alive
   * in the realm; dropping the map would make each `pages()` call return new objects.
   */
  #pages = new Map<string, Page>()
  readonly _timeoutSettings = new TimeoutSettings()

  constructor(guid: string, connection: Connection) {
    this.#guid = guid
    this.#connection = connection
  }

  /** Every open tab of the context, in BrowserOS's tab order; a page seen before keeps its
   * object. A live read, like every Neo getter. */
  pages(): Page[] {
    const listed = new Map<string, Page>()
    for (const created of get(this.#guid, 'pages')) {
      listed.set(
        created.guid,
        this.#pages.get(created.guid) ?? this.#build(created),
      )
    }
    this.#pages = listed
    return [...listed.values()]
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
    const known = this.#pages.get(guid)
    if (known) return known
    const page = this.#build(get(guid, 'initializer'))
    this.#pages.set(guid, page)
    return page
  }

  #build(created: CreatedPage): Page {
    return new Page(
      created.guid,
      created.mainFrame.guid,
      created.mainFrame.initializer.url,
      this,
      this.#connection,
    )
  }
}
