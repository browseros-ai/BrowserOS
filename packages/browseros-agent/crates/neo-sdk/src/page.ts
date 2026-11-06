import type { ByRoleOptions } from '../vendor/playwright/isomorphic/locatorUtils'
import type { BrowserContext } from './browser-context'
import type { Connection } from './connection'
import { type ClickOptions, Frame } from './frame'
import type { Locator, LocatorOptions } from './locator'
import { get } from './runtime-bridge'
import { TimeoutSettings } from './timeout-settings'

/**
 * Playwright's `Page`, backed by live reads from the tab this run is attached to. Its selector
 * builders and `click` forward to the main frame, as Playwright's do (`client/page.ts`).
 *
 * Known limitation: once the tab is closed, `url()` returns the last URL this object returned,
 * not the last URL the tab committed. Navigate `/a` to `/final`, close the tab before the
 * script calls `url()` again, and `url()` returns `/a`; Playwright returns `/final`. No CDP
 * source answers after a close, so matching Playwright here needs event tracking, which the
 * no-cache design leaves out (sync-getters-design.md, "Known limitation").
 */
export class Page {
  readonly #guid: string
  /**
   * The only state the SDK keeps. Playwright still answers `url()` after the page closes,
   * with the last URL, while every CDP source errors; seeded with the URL at run start.
   */
  #lastUrl: string
  readonly #context: BrowserContext
  /** Built once: `Locator` throws `… must belong to the same frame.` when two locators hold
   * different `Frame` objects, so `mainFrame()` must always return this one. */
  readonly #mainFrame: Frame
  readonly _timeoutSettings: TimeoutSettings

  constructor(
    guid: string,
    mainFrameGuid: string,
    url: string,
    context: BrowserContext,
    connection: Connection,
  ) {
    this.#guid = guid
    this.#lastUrl = url
    this.#context = context
    this._timeoutSettings = new TimeoutSettings(context._timeoutSettings)
    this.#mainFrame = new Frame(mainFrameGuid, this, connection)
  }

  url(): string {
    const url = get(this.#guid, 'url')
    if (url !== null) this.#lastUrl = url
    return this.#lastUrl
  }

  isClosed(): boolean {
    return get(this.#guid, 'isClosed')
  }

  mainFrame(): Frame {
    return this.#mainFrame
  }

  context(): BrowserContext {
    return this.#context
  }

  setDefaultTimeout(timeout: number): void {
    this._timeoutSettings.setDefaultTimeout(timeout)
  }

  /** Not strict, as Playwright's `page.click`, which forwards to the main frame. */
  async click(selector: string, options: ClickOptions = {}): Promise<void> {
    return await this.#mainFrame._click('page.click', selector, options)
  }

  locator(selector: string, options?: LocatorOptions): Locator {
    return this.mainFrame().locator(selector, options)
  }

  getByTestId(testId: string | RegExp): Locator {
    return this.mainFrame().getByTestId(testId)
  }

  getByAltText(text: string | RegExp, options?: { exact?: boolean }): Locator {
    return this.mainFrame().getByAltText(text, options)
  }

  getByLabel(text: string | RegExp, options?: { exact?: boolean }): Locator {
    return this.mainFrame().getByLabel(text, options)
  }

  getByPlaceholder(
    text: string | RegExp,
    options?: { exact?: boolean },
  ): Locator {
    return this.mainFrame().getByPlaceholder(text, options)
  }

  getByText(text: string | RegExp, options?: { exact?: boolean }): Locator {
    return this.mainFrame().getByText(text, options)
  }

  getByTitle(text: string | RegExp, options?: { exact?: boolean }): Locator {
    return this.mainFrame().getByTitle(text, options)
  }

  getByRole(role: string, options: ByRoleOptions = {}): Locator {
    return this.mainFrame().getByRole(role, options)
  }
}
