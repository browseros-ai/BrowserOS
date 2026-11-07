import type {
  LifecycleEvent,
  PageCloseParams,
  PageConsoleMessagesParams,
  PageEmulateMediaParams,
  PagePageErrorsParams,
} from '@browseros/neo-runtime-spec/serve'
import type { ByRoleOptions } from '../vendor/playwright/isomorphic/locatorUtils'
import { isString } from '../vendor/playwright/isomorphic/stringUtils'
import type { BrowserContext } from './browser-context'
import type { Connection } from './connection'
import { ConsoleMessage } from './console-message'
import {
  isTargetClosedError,
  parseSerializedError,
  targetClosedError,
} from './errors'
import { ChannelOwner, type EventTable } from './events'
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
  type PressOptions,
  type SelectOptionOptions,
  type TapOptions,
  type TextContentOptions,
  type UncheckOptions,
  type URLMatch,
  type WaitForLoadStateOptions,
  type WaitForURLOptions,
} from './frame'
import type { FrameLocator } from './frame-locator'
import type { Locator, LocatorOptions } from './locator'
import { serializeUrlMatch } from './neo-commands'
import type { Response } from './network'
import {
  PAGE_CLOSE_KEYS,
  PAGE_CONSOLE_MESSAGES_KEYS,
  PAGE_PAGE_ERRORS_KEYS,
  pickParams,
} from './protocol'
import { type CreatedFrame, get, matchesURL } from './runtime-bridge'
import type { SelectOptionValues } from './select-option'
import { TimeoutSettings } from './timeout-settings'
import {
  refuseSignal,
  Waiter,
  type WaitForEventOptions,
  wrapApiCall,
} from './waiter'

/**
 * Neo: the page's listener events, by Playwright's names (`Events.Page`): what Neo produces,
 * and Playwright's others, which other slices of this build add.
 */
const PAGE_EVENTS: EventTable = {
  receiver: 'Page',
  served: ['console', 'pageerror', 'dialog', 'dialogclosed', 'close', 'crash'],
  quiet: [],
  unserved: [
    'download',
    'filechooser',
    'domcontentloaded',
    'request',
    'response',
    'requestfailed',
    'requestfinished',
    'frameattached',
    'framedetached',
    'framenavigated',
    'load',
    'popup',
    'websocket',
    'worker',
  ],
}

/** Playwright's `Size`, a viewport's width and height in CSS pixels. */
export interface ViewportSize {
  width: number
  height: number
}

/** `emulateMedia`'s options: `null` stops emulating a feature. */
export interface EmulateMediaOptions {
  media?: 'screen' | 'print' | null
  colorScheme?: 'light' | 'dark' | 'no-preference' | null
  reducedMotion?: 'reduce' | 'no-preference' | null
  forcedColors?: 'active' | 'none' | null
  contrast?: 'no-preference' | 'more' | null
}

/** `consoleMessages` and `pageErrors`' options: Playwright's `ConsoleMessagesFilter`. */
export interface ConsoleMessagesOptions {
  filter?: 'all' | 'since-navigation'
}

/**
 * Playwright's `Page`, backed by live reads from its own tab, which its guid names. Its
 * selector builders, `goto`, `title`, `textContent`, `innerText`, `innerHTML`, `getAttribute`,
 * state reads, browser actions, `waitForURL` and `waitForLoadState` forward to the main frame, as
 * Playwright's do
 * (`client/page.ts`); `ariaSnapshot` snapshots the main frame's document. Only the run's
 * `BrowserContext` builds one, once per guid.
 *
 * Known limitation: once the tab is closed, `url()` returns the last URL this object returned,
 * not the last URL the tab committed. Navigate `/a` to `/final`, close the tab before the
 * script calls `url()` again, and `url()` returns `/a`; Playwright returns `/final`. No CDP
 * source answers after a close, so matching Playwright here needs event tracking, which the
 * no-cache design leaves out (sync-getters-design.md, "Known limitation").
 *
 * Deviation: `client/page.ts` `Page.constructor`: `this.once(Events.Page.Close, () => this._closedOrCrashedScope.close(this._closeErrorWithReason()));`
 * Playwright's page holds a `close` and a `crash` listener of its own from its first moment,
 * which `eventNames()` and `listenerCount` report; Neo's holds none, so a page nobody listens to
 * costs the run nothing.
 */
export class Page extends ChannelOwner {
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
  /** Playwright's `Page._viewportSize`: the page record's viewport (none for a tab of the run's
   * own context), then what `setViewportSize` set. */
  #viewportSize: ViewportSize | undefined
  /** Playwright's `Page._ownedContext`: the context `browser.newPage` made for this page. */
  _ownedContext: BrowserContext | undefined
  /** Whether `close` was emitted, so a close Neo reports itself (a context or browser closing)
   * and one Rust reports are one event. */
  #closeEmitted = false

  /** `guid` and `mainFrameGuid` come from one record Rust minted (`page@T`, `frame@T`), so a
   * page's calls and its frame's calls reach the same tab. */
  constructor(
    guid: string,
    mainFrameGuid: string,
    url: string,
    context: BrowserContext,
    connection: Connection,
    viewportSize?: ViewportSize,
  ) {
    super(guid, context._pipe, PAGE_EVENTS)
    this.#guid = guid
    this.#viewportSize = viewportSize
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
    // A page of a closed browser answers its last URL, as a closed tab does.
    if (this.#connection._disconnected) return this.#lastUrl
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
      // Playwright: `client/page.ts` `Page.close`: `await this._ownedContext.close();`
      if (this._ownedContext) await this._ownedContext.close()
      else if (options.runBeforeUnload)
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
    // Every page of a closed browser reads closed, as Playwright's do.
    if (this.#connection._disconnected) return true
    return get(this.#guid, 'isClosed')
  }

  /**
   * Playwright's `Page.setViewportSize`: the page's viewport and screen become `viewportSize`,
   * and `viewportSize()` answers it from then on, even when the call fails, as Playwright's.
   *
   * Deviation: `server/page.ts` `Page.setViewportSize`: `this._setEmulatedSize({ viewport: { ...viewportSize }, screen: { ...viewportSize } });`
   * Chromium drops the run session's override when the session detaches, so the page keeps the
   * size until the run ends; a later run starts on the tab's own size.
   */
  async setViewportSize(viewportSize: ViewportSize): Promise<void> {
    this.#viewportSize = viewportSize
    await this.#connection.sendPage(
      'page.setViewportSize',
      this.#guid,
      'setViewportSize',
      { viewportSize },
      0,
    )
  }

  /** Playwright's `Page.viewportSize`: `null` while the page emulates no viewport, which a tab
   * of the run's own context does until `setViewportSize`. */
  viewportSize(): ViewportSize | null {
    return this.#viewportSize || null
  }

  /**
   * Playwright's `Page.emulateMedia`: each option given replaces what the page emulates for it,
   * `null` stops emulating it, and an option left out keeps what it was.
   *
   * Deviation: `server/page.ts` `Page.emulateMedia`: `this._emulatedMedia.media = options.media;`
   * The emulation lasts until the run ends (its session detaches), not for the page's life.
   */
  async emulateMedia(options: EmulateMediaOptions = {}): Promise<void> {
    await this.#connection.sendPage(
      'page.emulateMedia',
      this.#guid,
      'emulateMedia',
      {
        media: options.media === null ? 'no-override' : options.media,
        colorScheme:
          options.colorScheme === null ? 'no-override' : options.colorScheme,
        reducedMotion:
          options.reducedMotion === null
            ? 'no-override'
            : options.reducedMotion,
        forcedColors:
          options.forcedColors === null ? 'no-override' : options.forcedColors,
        contrast: options.contrast === null ? 'no-override' : options.contrast,
      } as PageEmulateMediaParams,
      0,
    )
  }

  /** Playwright's `Page.requestGC`: a full garbage collection in the page's renderer, which
   * frees every object nothing reaches any more. */
  async requestGC(): Promise<void> {
    await this.#connection.sendPage(
      'page.requestGC',
      this.#guid,
      'requestGC',
      {},
      0,
    )
  }

  /**
   * Playwright's `Page.pause`: its default timeouts are off while the context's `pause` call
   * runs, which the context's debugger holds only once it is enabled (`context.debugger`'s
   * `requestPause`, `next` or `runTo`), until it is resumed; otherwise the call returns at once,
   * as Playwright's does with a headless browser and no inspector.
   *
   * Deviation: `client/page.ts` `Page.pause`: `await this._closedOrCrashedScope.safeRace(this.context()._channel.pause({}, kNoTimeout));`
   * With a headed browser Playwright opens its Inspector and waits for a human to resume. A Neo
   * run is unattended and has no Inspector, so `pause()` never waits, headed or not.
   */
  async pause(_options?: unknown): Promise<void> {
    const settings = this.#context._timeoutSettings
    const defaultNavigationTimeout = settings.defaultNavigationTimeout()
    const defaultTimeout = settings.defaultTimeout()
    this.#context.setDefaultNavigationTimeout(0)
    this.#context.setDefaultTimeout(0)
    try {
      await this.#connection.sendContext(
        'page.pause',
        this.#context._guid,
        'pause',
        {},
        0,
      )
    } finally {
      this.#context.setDefaultNavigationTimeout(defaultNavigationTimeout)
      this.#context.setDefaultTimeout(defaultTimeout)
    }
  }

  /** Playwright's page channel handlers (`client/page.ts` constructor). */
  _onEvent(method: string): void {
    if (method === 'close') this._onClose()
    else if (method === 'crash') this.emit('crash', this)
  }

  /** Playwright's `Page._onClose`: the page's `close`, then the context's `pageclose`, once. */
  _onClose(): void {
    if (this.#closeEmitted) return
    this.#closeEmitted = true
    this.emit('close', this)
    this.#context.emit('pageclose', this)
  }

  /**
   * Playwright's `Page.waitForEvent`: resolves with the first `event` its predicate (sync or
   * async) accepts. It rejects at the timeout (the page's default, 30 s unless set), and when
   * the page crashes or closes first, unless that is the event awaited.
   */
  async waitForEvent(
    event: string,
    optionsOrPredicate: WaitForEventOptions = {},
  ): Promise<unknown> {
    return await wrapApiCall('page.waitForEvent', () =>
      this._waitForEvent(
        event,
        optionsOrPredicate,
        `waiting for event "${event}"`,
      ),
    )
  }

  /** Playwright's `Page._waitForEvent`, which its `waitFor*` helpers share. */
  async _waitForEvent(
    event: string,
    optionsOrPredicate: WaitForEventOptions,
    logLine?: string,
  ): Promise<unknown> {
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
    if (logLine) waiter.log(logLine)
    waiter.rejectOnTimeout(
      timeout,
      `Timeout ${timeout}ms exceeded while waiting for event "${event}"`,
    )
    if (event !== 'crash')
      waiter.rejectOnEvent(this, 'crash', new Error('Page crashed'))
    if (event !== 'close')
      waiter.rejectOnEvent(this, 'close', () => targetClosedError())
    const result = await waiter.waitForEvent(this, event, predicate)
    waiter.dispose()
    return result
  }

  /**
   * The page's console messages, the last 200 as Playwright keeps them: since the last
   * main-frame navigation by default, every kept one with `filter: 'all'`. Messages the run's
   * listeners already saw are included.
   *
   * Deviation: `server/page.ts` `Page.addConsoleMessage`: `this._consoleMessages.push(message);`
   * Playwright's server keeps a page's messages and errors from the page's first moment. Neo
   * keeps them from the run's first look (a listener, or a read like this one), starting with
   * what the browser kept of the current document; the browser keeps nothing of a document the
   * page left before then, so `'all'` does not reach back past it.
   */
  async consoleMessages(
    options?: ConsoleMessagesOptions,
  ): Promise<ConsoleMessage[]> {
    const { messages } = await this.#connection.sendPage(
      'page.consoleMessages',
      this.#guid,
      'consoleMessages',
      pickParams(PAGE_CONSOLE_MESSAGES_KEYS, {
        filter: options?.filter,
      }) as PageConsoleMessagesParams,
      0,
    )
    return messages.map((message) => new ConsoleMessage(message, this, null))
  }

  async clearConsoleMessages(): Promise<void> {
    await this.#connection.sendPage(
      'page.clearConsoleMessages',
      this.#guid,
      'clearConsoleMessages',
      {},
      0,
    )
  }

  /** The page's uncaught exceptions as `Error`s, kept as {@link Page.consoleMessages} keeps
   * messages. */
  async pageErrors(options?: ConsoleMessagesOptions): Promise<Error[]> {
    const { errors } = await this.#connection.sendPage(
      'page.pageErrors',
      this.#guid,
      'pageErrors',
      pickParams(PAGE_PAGE_ERRORS_KEYS, {
        filter: options?.filter,
      }) as PagePageErrorsParams,
      0,
    )
    return errors.map((error) => parseSerializedError(error))
  }

  async clearPageErrors(): Promise<void> {
    await this.#connection.sendPage(
      'page.clearPageErrors',
      this.#guid,
      'clearPageErrors',
      {},
      0,
    )
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
   * The main frame's {@link Frame.waitForURL}: waits until the page's URL matches `url` (a glob
   * string or a `RegExp`; a predicate function throws), then until `waitUntil` (default `load`;
   * `networkidle` throws) fires, at once if the current document already fired it, and resolves
   * `undefined`.
   */
  async waitForURL(url: URLMatch, options?: WaitForURLOptions): Promise<void> {
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
   * The main frame's accessibility tree as Playwright's YAML. With `mode: 'ai'`, each rendered
   * iframe's document appears under its `iframe` node, as in Playwright, and nodes carry refs that
   * `page.locator('aria-ref=…')` resolves, in this run or a later one, until that document's next
   * snapshot or navigation. A ref inside a frame is its path from the main frame:
   * `- iframe [ref=e7]:` holds `- textbox "Email" [ref=f7e3]`, and a frame inside it adds a step
   * (`f7f2e1`); Playwright numbers frames instead (`f1e3`). The default mode prints iframes without
   * their content, as Playwright's does. `depth` limits tree levels (0 means unlimited).
   * For a section, use `locator.ariaSnapshot()`. AI mode does not auto-wait for that locator,
   * so use `locator.waitFor()` first if needed.
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
