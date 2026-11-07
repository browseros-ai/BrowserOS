import type {
  LifecycleEvent,
  PageCloseParams,
  PageGoBackParams,
  PageGoForwardParams,
  PageReloadParams,
  ResponseChannel,
} from '@browseros/neo-runtime-spec/serve'
import type { ByRoleOptions } from '../vendor/playwright/isomorphic/locatorUtils'
import { isString } from '../vendor/playwright/isomorphic/stringUtils'
import type { BrowserContext } from './browser-context'
import type { Connection } from './connection'
import { isTargetClosedError } from './errors'
import {
  type AriaSnapshotOptions,
  type CheckOptions,
  type ClickOptions,
  type DblclickOptions,
  type FillOptions,
  Frame,
  type GetAttributeOptions,
  type GotoOptions,
  type HoverOptions,
  type InnerHTMLOptions,
  type InnerTextOptions,
  type InputValueOptions,
  type IsCheckedOptions,
  type IsDisabledOptions,
  type IsEditableOptions,
  type IsEnabledOptions,
  type IsHiddenOptions,
  type IsVisibleOptions,
  type NavigationURLMatch,
  type PressOptions,
  type SelectOptionOptions,
  type TapOptions,
  type TextContentOptions,
  type UncheckOptions,
  type URLMatch,
  verifyLoadState,
  type WaitForLoadStateOptions,
  type WaitForNavigationOptions,
  type WaitForURLOptions,
} from './frame'
import type { FrameLocator } from './frame-locator'
import type { Locator, LocatorOptions } from './locator'
import { serializeUrlMatch } from './neo-commands'
import { type Response, responseFor } from './network'
import {
  PAGE_CLOSE_KEYS,
  PAGE_GO_BACK_KEYS,
  PAGE_GO_FORWARD_KEYS,
  PAGE_RELOAD_KEYS,
  pickParams,
} from './protocol'
import { type CreatedFrame, get, matchesURL } from './runtime-bridge'
import type { SelectOptionValues } from './select-option'
import { TimeoutSettings } from './timeout-settings'

/** `reload`, `goBack` and `goForward`'s options as a script passes them: the wire params plus
 * `timeout`, as Playwright's `channels.PageReloadOptions & TimeoutOptions`. */
export type HistoryOptions = { waitUntil?: LifecycleEvent; timeout?: number }

/**
 * Playwright's `Page`, backed by live reads from its own tab, which its guid names. Its
 * selector builders, `goto`, `title`, `textContent`, `innerText`, `innerHTML`, `getAttribute`,
 * state reads, browser actions, `waitForURL`, `waitForLoadState` and `waitForNavigation` forward
 * to the main frame, as Playwright's do (`client/page.ts`); `ariaSnapshot` snapshots the main
 * frame's document; `reload`, `goBack` and `goForward` are the page's own commands. Only the run's
 * `BrowserContext` builds one, once per guid.
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
   * The only tab state a `Page` keeps. Playwright still answers `url()` after the page closes,
   * with the last URL, while every CDP source errors; seeded with the tab's URL in the record
   * the run was handed (`RunInit.url` for the run page).
   */
  #lastUrl: string
  readonly #context: BrowserContext
  readonly #connection: Connection
  /** Built once: `Locator` throws `… must belong to the same frame.` when two locators hold
   * different `Frame` objects, so `mainFrame()` must always return this one. */
  readonly #mainFrame: Frame
  /** Stable identity, including detached frames a script may still hold. */
  readonly #frames = new Map<string, Frame>()
  readonly _timeoutSettings: TimeoutSettings

  /** `guid` and `mainFrameGuid` come from one record Rust minted (`page@T`, `frame@T`), so a
   * page's calls and its frame's calls reach the same tab. */
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
    this.#connection = connection
    this._timeoutSettings = new TimeoutSettings(context._timeoutSettings)
    this.#mainFrame = new Frame(mainFrameGuid, this, connection, {
      url,
      name: '',
    })
    this.#frames.set(mainFrameGuid, this.#mainFrame)
  }

  url(): string {
    const url = get(this.#guid, 'url')
    if (url !== null) this.#lastUrl = url
    return this.#lastUrl
  }

  /** Makes this the active tab of its window. */
  async bringToFront(): Promise<void> {
    await this.#connection.sendPage(
      'page.bringToFront',
      this.#guid,
      'bringToFront',
      {},
      0,
    )
  }

  /**
   * Playwright's client `close`. By default it closes the tab and resolves once the tab is
   * gone, so `isClosed()` is then true; closing a closed page resolves. `runBeforeUnload` asks
   * the page to close itself instead, which runs its `beforeunload` handlers and resolves
   * without waiting for the close; on a closed page it throws. Both calls have no timeout.
   *
   * Playwright also records `reason` and that `close` was called, for its event waiters and
   * route handlers. Neo has neither: the reason goes to Rust.
   */
  async close(
    options: { runBeforeUnload?: boolean; reason?: string } = {},
  ): Promise<void> {
    try {
      if (options.runBeforeUnload)
        await this.#connection.sendPage(
          'page.close',
          this.#guid,
          'runBeforeUnload',
          {},
          0,
        )
      else
        await this.#connection.sendPage(
          'page.close',
          this.#guid,
          'close',
          pickParams(PAGE_CLOSE_KEYS, {
            reason: options.reason,
          }) as PageCloseParams,
          0,
        )
    } catch (error) {
      if (isTargetClosedError(error) && !options.runBeforeUnload) return
      throw error
    }
  }

  isClosed(): boolean {
    return get(this.#guid, 'isClosed')
  }

  mainFrame(): Frame {
    return this.#mainFrame
  }

  /** Interns records from every getter so frame comparisons remain valid across reads. */
  _frameFrom(record: CreatedFrame): Frame {
    let frame = this.#frames.get(record.guid)
    if (!frame) {
      frame = new Frame(record.guid, this, this.#connection, record.initializer)
      this.#frames.set(record.guid, frame)
    } else frame._updateInitializer(record.initializer)
    return frame
  }

  frames(): Frame[] {
    return get(this.#guid, 'frames').map((record) => this._frameFrom(record))
  }

  /** As Playwright, a nonempty name takes precedence when both selectors are given. */
  frame(
    frameSelector: string | { name?: string; url?: URLMatch },
  ): Frame | null {
    const name = isString(frameSelector) ? frameSelector : frameSelector.name
    const url =
      typeof frameSelector === 'object' ? frameSelector.url : undefined
    if (!name && !url)
      throw new Error('Either name or url matcher should be specified')
    const matcher = name ? undefined : serializeUrlMatch('page.frame', url)
    // Neo: Reuses one live frame-tree reply and refreshes its canonical Frame objects.
    const records = get(this.#guid, 'frames')
    const frames = records.map((record) => this._frameFrom(record))
    const index = records.findIndex(({ initializer }) =>
      matcher
        ? matchesURL(initializer.url, matcher)
        : initializer.name === name,
    )
    return frames[index] ?? null
  }

  context(): BrowserContext {
    return this.#context
  }

  setDefaultTimeout(timeout: number): void {
    this._timeoutSettings.setDefaultTimeout(timeout)
  }

  setDefaultNavigationTimeout(timeout: number): void {
    this._timeoutSettings.setDefaultNavigationTimeout(timeout)
  }

  /** The navigation's `Response`, or `null` where Playwright's has none. */
  async goto(url: string, options: GotoOptions = {}): Promise<Response | null> {
    return await this.#mainFrame._goto('page.goto', url, options)
  }

  async title(): Promise<string> {
    return await this.#mainFrame._title('page.title')
  }

  /**
   * Reloads the page and resolves the new document's main `Response` once `waitUntil` (default
   * `load`; also `domcontentloaded` or `commit`) fires for it, as Playwright's `page.reload`.
   * A failed reload rejects with its network error. Runs under the navigation timeout.
   * Neo limit: `networkidle` throws an `Error`.
   */
  async reload(options: HistoryOptions = {}): Promise<Response | null> {
    const waitUntil = verifyLoadState(
      'waitUntil',
      options.waitUntil === undefined ? 'load' : options.waitUntil,
    )
    const result = await this.#connection.sendPage(
      'page.reload',
      this.#guid,
      'reload',
      pickParams(PAGE_RELOAD_KEYS, {
        ...options,
        waitUntil,
      }) as PageReloadParams,
      this._timeoutSettings.navigationTimeout(options),
    )
    return this.#response(result?.response)
  }

  /**
   * Navigates to the previous page in history, as Playwright's `page.goBack`: resolves `null` at
   * once when there is none; otherwise, once `waitUntil` fires, the new document's `Response`,
   * or `null` for a same-document entry or a document restored from the back-forward cache, which
   * counts as loaded. A failed navigation rejects. Options and limits as `reload`'s.
   */
  async goBack(options: HistoryOptions = {}): Promise<Response | null> {
    const waitUntil = verifyLoadState(
      'waitUntil',
      options.waitUntil === undefined ? 'load' : options.waitUntil,
    )
    const result = await this.#connection.sendPage(
      'page.goBack',
      this.#guid,
      'goBack',
      pickParams(PAGE_GO_BACK_KEYS, {
        ...options,
        waitUntil,
      }) as PageGoBackParams,
      this._timeoutSettings.navigationTimeout(options),
    )
    return this.#response(result?.response)
  }

  /** {@link Page.goBack} toward the next page in history. */
  async goForward(options: HistoryOptions = {}): Promise<Response | null> {
    const waitUntil = verifyLoadState(
      'waitUntil',
      options.waitUntil === undefined ? 'load' : options.waitUntil,
    )
    const result = await this.#connection.sendPage(
      'page.goForward',
      this.#guid,
      'goForward',
      pickParams(PAGE_GO_FORWARD_KEYS, {
        ...options,
        waitUntil,
      }) as PageGoForwardParams,
      this._timeoutSettings.navigationTimeout(options),
    )
    return this.#response(result?.response)
  }

  /** The `Response` a history command answered, built once per guid, or `null`. */
  #response(response: ResponseChannel | undefined): Response | null {
    return response
      ? responseFor(this.#connection, response.guid, this.#mainFrame)
      : null
  }

  /** The main frame's {@link Frame.waitForNavigation}. */
  async waitForNavigation(
    options: WaitForNavigationOptions = {},
  ): Promise<Response | null> {
    return await this.#mainFrame._waitForNavigation(
      'page.waitForNavigation',
      options,
    )
  }

  /**
   * The main frame's {@link Frame.waitForURL}: waits until the page's URL matches `url` (a glob
   * string, a `RegExp` or a predicate on its `URL`), then until `waitUntil` (default `load`;
   * `networkidle` throws) fires, at once if the current document already fired it, and resolves
   * `undefined`.
   */
  async waitForURL(
    url: NavigationURLMatch,
    options?: WaitForURLOptions,
  ): Promise<void> {
    return await this.#mainFrame._waitForURL('page.waitForURL', url, options)
  }

  /**
   * The main frame's {@link Frame.waitForLoadState}: waits until `state` (default `load`;
   * `networkidle` throws) fired for the current or next document, and resolves `undefined`.
   */
  async waitForLoadState(
    state?: LifecycleEvent,
    options?: WaitForLoadStateOptions,
  ): Promise<void> {
    return await this.#mainFrame._waitForLoadState(
      'page.waitForLoadState',
      state,
      options,
    )
  }

  /** Not strict, as Playwright's `page.textContent`, which forwards to the main frame. */
  async textContent(
    selector: string,
    options: TextContentOptions = {},
  ): Promise<string | null> {
    return await this.#mainFrame._textContent(
      'page.textContent',
      selector,
      options,
    )
  }

  async innerText(
    selector: string,
    options: InnerTextOptions = {},
  ): Promise<string> {
    return await this.#mainFrame._innerText('page.innerText', selector, options)
  }

  async innerHTML(
    selector: string,
    options: InnerHTMLOptions = {},
  ): Promise<string> {
    return await this.#mainFrame._innerHTML('page.innerHTML', selector, options)
  }

  async getAttribute(
    selector: string,
    name: string,
    options: GetAttributeOptions = {},
  ): Promise<string | null> {
    return await this.#mainFrame._getAttribute(
      'page.getAttribute',
      selector,
      name,
      options,
    )
  }

  /**
   * The main frame's accessibility tree as YAML. Default mode omits refs and prints iframes
   * without their content, as Playwright's does. `mode: 'ai'` adds `[ref=eN]` refs that
   * `page.locator('aria-ref=…')` resolves, in this run or a later one, until that document's
   * next snapshot or navigation, and puts each rendered iframe's document under its `iframe`
   * node, as in Playwright. A ref inside a frame is its path from the main frame:
   * `- iframe [ref=e7]:` holds `- textbox "Email" [ref=f7e3]`, and a frame inside it adds a step
   * (`f7f2e1`); Playwright numbers frames instead (`f1e3`). `depth` limits tree levels (0 means
   * unlimited). For a section, use `locator.ariaSnapshot()`: AI mode does not auto-wait for that
   * locator, so use `locator.waitFor()` first if needed.
   */
  async ariaSnapshot(options: AriaSnapshotOptions = {}): Promise<string> {
    return await this.#mainFrame._ariaSnapshot('page.ariaSnapshot', options)
  }

  /** Not strict, as Playwright's `page.click`, which forwards to the main frame. */
  async click(selector: string, options: ClickOptions = {}): Promise<void> {
    return await this.#mainFrame._click('page.click', selector, options)
  }

  /** Not strict, as Playwright's `page.dblclick`, which forwards to the main frame. */
  async dblclick(
    selector: string,
    options: DblclickOptions = {},
  ): Promise<void> {
    return await this.#mainFrame._dblclick('page.dblclick', selector, options)
  }

  /** Not strict, as Playwright's `page.hover`, which forwards to the main frame. */
  async hover(selector: string, options: HoverOptions = {}): Promise<void> {
    return await this.#mainFrame._hover('page.hover', selector, options)
  }

  /** Not strict, as Playwright's `page.tap`, which forwards to the main frame. */
  async tap(selector: string, options: TapOptions = {}): Promise<void> {
    return await this.#mainFrame._tap('page.tap', selector, options)
  }

  async fill(
    selector: string,
    value: string,
    options: FillOptions = {},
  ): Promise<void> {
    return await this.#mainFrame._fill('page.fill', selector, value, options)
  }

  async press(
    selector: string,
    key: string,
    options: PressOptions = {},
  ): Promise<void> {
    return await this.#mainFrame._press('page.press', selector, key, options)
  }

  async inputValue(
    selector: string,
    options: InputValueOptions = {},
  ): Promise<string> {
    return await this.#mainFrame._inputValue(
      'page.inputValue',
      selector,
      options,
    )
  }

  async isVisible(
    selector: string,
    options: IsVisibleOptions = {},
  ): Promise<boolean> {
    return await this.#mainFrame._isVisible('page.isVisible', selector, options)
  }

  async isChecked(
    selector: string,
    options: IsCheckedOptions = {},
  ): Promise<boolean> {
    return await this.#mainFrame._isChecked('page.isChecked', selector, options)
  }

  async isHidden(
    selector: string,
    options: IsHiddenOptions = {},
  ): Promise<boolean> {
    return await this.#mainFrame._isHidden('page.isHidden', selector, options)
  }

  async isEnabled(
    selector: string,
    options: IsEnabledOptions = {},
  ): Promise<boolean> {
    return await this.#mainFrame._isEnabled('page.isEnabled', selector, options)
  }

  async isDisabled(
    selector: string,
    options: IsDisabledOptions = {},
  ): Promise<boolean> {
    return await this.#mainFrame._isDisabled(
      'page.isDisabled',
      selector,
      options,
    )
  }

  async isEditable(
    selector: string,
    options: IsEditableOptions = {},
  ): Promise<boolean> {
    return await this.#mainFrame._isEditable(
      'page.isEditable',
      selector,
      options,
    )
  }

  async check(selector: string, options: CheckOptions = {}): Promise<void> {
    return await this.#mainFrame._check('page.check', selector, options)
  }

  async uncheck(selector: string, options: UncheckOptions = {}): Promise<void> {
    return await this.#mainFrame._uncheck('page.uncheck', selector, options)
  }

  async selectOption(
    selector: string,
    values: SelectOptionValues,
    options: SelectOptionOptions = {},
  ): Promise<string[]> {
    return await this.#mainFrame._selectOption(
      'page.selectOption',
      selector,
      values,
      options,
    )
  }

  async setChecked(
    selector: string,
    checked: boolean,
    options: CheckOptions = {},
  ): Promise<void> {
    if (checked)
      await this.#mainFrame._check('page.setChecked', selector, options)
    else await this.#mainFrame._uncheck('page.setChecked', selector, options)
  }

  locator(selector: string, options?: LocatorOptions): Locator {
    return this.mainFrame().locator(selector, options)
  }

  /** The element an `ai` snapshot named `ref` (`page.ariaSnapshot({ mode: 'ai' })`), until a
   * later snapshot of its document or a navigation renames it. */
  getByRef(ref: string): Locator {
    return this.locator(`aria-ref=${ref}`)
  }

  frameLocator(selector?: string): FrameLocator {
    return this.mainFrame().frameLocator(selector)
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

  /** Playwright's `ChannelOwner.toJSON`: `JSON.stringify(page)`, and a script that returns its
   * page, give its type and guid only, never this object's internals. */
  toJSON(): { _type: 'Page'; _guid: string } {
    return { _type: 'Page', _guid: this.#guid }
  }
}
