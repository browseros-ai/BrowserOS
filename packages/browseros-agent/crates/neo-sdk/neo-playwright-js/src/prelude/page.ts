/**
 * Playwright's Page over one tab.
 *
 * Playwright's synchronous accessors (`url()`, `isClosed()`, `frames()`, `mainFrame()`) read
 * caches here. They are refreshed by the host's page facts after every op, by the URL and
 * frame tree each navigation or wait returns, and by events (a close, a popup). The run's
 * own page may not have a tab yet (a new task). Its ops then name it as
 * `{"$neo":"page","tab": null}`, claw-sdk's door opens one tab for them however many race to be
 * first, and the reply's `defaultPage` binds this object to that tab (`BrowserContext`).
 */
import { invoke, invokeObserved, sleep } from './bridge'
import type { BrowserContext } from './context'
import { ElementHandle, elementBrand } from './element-handle'
import { EventEmitter } from './emitter'
import {
  abortedError,
  clientError,
  plainError,
  prefixed,
  unsupportedError,
} from './errors'
import {
  assertMaxArguments,
  type EvaluateOptions,
  pageCall,
} from './evaluation'
import { EventDeadline } from './event-deadline'
import { checkReported, isReported } from './events'
import {
  type ClickOptions,
  type ExactOption,
  Frame,
  frameMatches,
  knownLoadState,
  loadState,
  loadStateError,
  type NavigateOptions,
  type ScreenshotOptionsInput,
  type ScriptTagOptions,
  type SnapshotOptions,
  type StrictOption,
  type StyleTagOptions,
  type TextMatch,
  type UrlMatcher,
  type WaitForFunctionOptions,
  type WaitForNavigationOptions,
  type WaitForSelectorOptions,
  type WaitHandle,
} from './frame'
import { Keyboard, Mouse } from './input'
import type { FrameLocator, Locator } from './locator'
import { markHandle, pick } from './options'
import { Response } from './response'
import type { LocatorOptions } from './selectors'
import {
  type SignalLike,
  type SignalOption,
  signalOf,
  untilAborted,
} from './signal'
import { type TimeoutOption, TimeoutSettings } from './timeouts'
import {
  type AriaSnapshotJSON,
  type ByRoleOptions,
  escapeRegExp,
} from './vendored'
import type {
  DragOptions,
  FillOptions,
  FrameInfo,
  KeyOptions,
  LoadState,
  NavigationResult,
  Observation,
  PageBrand,
  PageFact,
  PageHandle,
} from './wire'

/** Page-scoped preferences; omitted values are retained and null clears just that override. */
export type MediaOptions = {
  media?: 'screen' | 'print' | null
  colorScheme?: 'dark' | 'light' | 'no-preference' | null
  reducedMotion?: 'reduce' | 'no-preference' | null
  forcedColors?: 'active' | 'none' | null
  contrast?: 'no-preference' | 'more' | null
}

export type CloseOptions = { runBeforeUnload?: boolean; reason?: string }

const PDF_KEYS = [
  'scale',
  'displayHeaderFooter',
  'headerTemplate',
  'footerTemplate',
  'printBackground',
  'landscape',
  'pageRanges',
  'format',
  'width',
  'height',
  'preferCSSPageSize',
  'margin',
  'tagged',
  'outline',
] as const

export class Page extends EventEmitter {
  /** The browser's tab id, which the page's brand carries; null until a fact or result names it. */
  _tab: number | null
  readonly _context: BrowserContext
  readonly _timeoutSettings: TimeoutSettings
  readonly _mainFrame: Frame
  readonly keyboard: Keyboard
  readonly mouse: Mouse
  _url: string
  /** Historical opening events fill an unknown URL once. Live observations keep it current. */
  #urlKnown: boolean
  _title = ''
  _closed = false
  /** The newest host read applied to this tab. */
  #observedSeq = 0
  /** Close events recorded through the last live observation belong to its earlier life. */
  #liveHead = 0
  /** A close can reach both a waiter and the event pump, including after re-admission. */
  #closeEvent = 0
  /** Host-approved close, deferred until that record's position in its own reply. */
  #approvedClose: { seq: number; observed: number } | null = null
  /**
   * Whether `waitForTimeout` reads this page's frame tree again (FR-1): the run's page, a page
   * the run opened or learned of after it started, and a tab the run's init listed once the
   * script ran an op on it. A listed tab the script never touched keeps the tree it had.
   */
  _working = false
  /**
   * Navigation ops (goto, reload, goBack, goForward) in flight on this page. While Chromium has
   * not answered a `Page.navigate` (or history or reload command) for a tab, it holds every
   * command bound for that tab's renderer, from any session, until the navigation commits or
   * fails (measured: `Page.getFrameTree` and `Runtime.evaluate` both wait). Upstream's
   * `waitForTimeout` never reads the page, so its frame refresh skips a page with one in flight
   * rather than hold the script's sleep until the page answers, which deadlocks a script that
   * releases the navigation once it has slept. The navigation's own reply refreshes the tree.
   */
  _navigationsInFlight = 0
  readonly #childFrames = new Map<string, Frame>()

  constructor(context: BrowserContext, tab: number | null, url?: string) {
    super()
    this._context = context
    this._tab = tab
    this._url = url ?? 'about:blank'
    this.#urlKnown = url !== undefined
    this._timeoutSettings = new TimeoutSettings(context._timeoutSettings)
    this._mainFrame = new Frame(this, null, null)
    this.keyboard = new Keyboard(this)
    this.mouse = new Mouse(this)
    markHandle(this)
  }

  // ---- synchronous state ----------------------------------------------------------------

  context(): BrowserContext {
    return this._context
  }

  mainFrame(): Frame {
    return this._mainFrame
  }

  frames(): Frame[] {
    return [this._mainFrame, ...this.#childFrames.values()]
  }

  frame(selector: string | { name?: string; url?: UrlMatcher }): Frame | null {
    const name = typeof selector === 'string' ? selector : selector.name
    const url = typeof selector === 'string' ? undefined : selector.url
    if (!name && url === undefined)
      throw plainError(
        'page.frame',
        'Either name or url matcher should be specified',
      )
    return this.frames().find((frame) => frameMatches(frame, selector)) ?? null
  }

  url(): string {
    return this._url
  }

  isClosed(): boolean {
    return this._closed
  }

  setDefaultTimeout(timeout: number): void {
    this._timeoutSettings.setDefaultTimeout(timeout)
  }

  setDefaultNavigationTimeout(timeout: number): void {
    this._timeoutSettings.setDefaultNavigationTimeout(timeout)
  }

  /** No video is recorded, which Playwright reports as null. */
  video(): null {
    return null
  }

  /** The hosted viewport is fixed by Neo, not set through Playwright. */
  viewportSize(): null {
    return null
  }

  workers(): never[] {
    return []
  }

  /**
   * The brand the bootstrap's codec sends for a page the script passes to a call itself
   * (`neo.solveCaptcha(page)`), the handle the prelude's own ops send. A page whose tab no
   * reply named carries `tab: null`, which the door refuses with a hint.
   */
  toJSON(): PageHandle {
    return this._tab === null
      ? { $neo: 'page', tab: null }
      : { $neo: 'page', tab: this._tab }
  }

  // ---- builders ---------------------------------------------------------------------------

  locator(selector: string, options?: LocatorOptions): Locator {
    return this._mainFrame._locator('page.locator', selector, options)
  }

  /** Playwright's own definition (client/page.ts 1.64): a locator over Neo's ref engine. */
  getByRef(ref: string): Locator {
    return this.locator(`aria-ref=${ref}`)
  }

  getByTestId(testId: TextMatch): Locator {
    return this._mainFrame.getByTestId(testId)
  }

  getByAltText(text: TextMatch, options?: ExactOption): Locator {
    return this._mainFrame.getByAltText(text, options)
  }

  getByLabel(text: TextMatch, options?: ExactOption): Locator {
    return this._mainFrame.getByLabel(text, options)
  }

  getByPlaceholder(text: TextMatch, options?: ExactOption): Locator {
    return this._mainFrame.getByPlaceholder(text, options)
  }

  getByText(text: TextMatch, options?: ExactOption): Locator {
    return this._mainFrame.getByText(text, options)
  }

  getByTitle(text: TextMatch, options?: ExactOption): Locator {
    return this._mainFrame.getByTitle(text, options)
  }

  getByRole(role: string, options: ByRoleOptions = {}): Locator {
    return this._mainFrame.getByRole(role, options)
  }

  frameLocator(selector?: string): FrameLocator {
    return this._mainFrame.frameLocator(selector)
  }

  // ---- shortcuts on the main frame --------------------------------------------------------

  click(selector: string, options?: ClickOptions): Promise<void> {
    return this._mainFrame._click('page.click', selector, options)
  }

  dblclick(selector: string, options?: ClickOptions): Promise<void> {
    return this._mainFrame._dblclick('page.dblclick', selector, options)
  }

  hover(selector: string, options?: ClickOptions): Promise<void> {
    return this._mainFrame._hover('page.hover', selector, options)
  }

  check(selector: string, options?: ClickOptions): Promise<void> {
    return this._mainFrame._check('page.check', selector, true, options)
  }

  uncheck(selector: string, options?: ClickOptions): Promise<void> {
    return this._mainFrame._check('page.uncheck', selector, false, options)
  }

  setChecked(
    selector: string,
    checked: boolean,
    options?: ClickOptions,
  ): Promise<void> {
    return this._mainFrame._check('page.setChecked', selector, checked, options)
  }

  dragAndDrop(
    source: string,
    target: string,
    options?: DragOptions & TimeoutOption & StrictOption,
  ): Promise<void> {
    return this._mainFrame._dragAndDrop(
      'page.dragAndDrop',
      source,
      target,
      options,
    )
  }

  fill(
    selector: string,
    value: string,
    options?: FillOptions & TimeoutOption & StrictOption,
  ): Promise<void> {
    return this._mainFrame._fill('page.fill', selector, value, options)
  }

  press(
    selector: string,
    key: string,
    options?: KeyOptions & TimeoutOption & StrictOption,
  ): Promise<void> {
    return this._mainFrame._press('page.press', selector, key, options)
  }

  type(
    selector: string,
    text: string,
    options?: KeyOptions & TimeoutOption & StrictOption,
  ): Promise<void> {
    return this._mainFrame._type('page.type', selector, text, options)
  }

  focus(
    selector: string,
    options?: TimeoutOption & StrictOption,
  ): Promise<void> {
    return this._mainFrame._focus('page.focus', selector, options)
  }

  selectOption(
    selector: string,
    values: unknown,
    options?: TimeoutOption & StrictOption & { force?: boolean },
  ): Promise<string[]> {
    return this._mainFrame._selectOption(
      'page.selectOption',
      selector,
      values,
      options,
    )
  }

  setInputFiles(
    selector: string,
    files: unknown,
    options?: TimeoutOption & StrictOption,
  ): Promise<void> {
    return this._mainFrame._setInputFiles(
      'page.setInputFiles',
      selector,
      files,
      options,
    )
  }

  dispatchEvent(
    selector: string,
    type: string,
    eventInit?: unknown,
    options?: TimeoutOption & StrictOption,
  ): Promise<void> {
    return this._mainFrame._dispatchEvent(
      'page.dispatchEvent',
      selector,
      type,
      eventInit,
      options,
    )
  }

  textContent(
    selector: string,
    options?: TimeoutOption & StrictOption,
  ): Promise<string | null> {
    return this._mainFrame._read(
      'locator.textContent',
      'page.textContent',
      selector,
      options,
    )
  }

  innerText(
    selector: string,
    options?: TimeoutOption & StrictOption,
  ): Promise<string> {
    return this._mainFrame._read(
      'locator.innerText',
      'page.innerText',
      selector,
      options,
    )
  }

  innerHTML(
    selector: string,
    options?: TimeoutOption & StrictOption,
  ): Promise<string> {
    return this._mainFrame._read(
      'locator.innerHTML',
      'page.innerHTML',
      selector,
      options,
    )
  }

  getAttribute(
    selector: string,
    name: string,
    options?: TimeoutOption & StrictOption,
  ): Promise<string | null> {
    return this._mainFrame._getAttribute(
      'page.getAttribute',
      selector,
      name,
      options,
    )
  }

  inputValue(
    selector: string,
    options?: TimeoutOption & StrictOption,
  ): Promise<string> {
    return this._mainFrame._read(
      'locator.inputValue',
      'page.inputValue',
      selector,
      options,
    )
  }

  isChecked(
    selector: string,
    options?: TimeoutOption & StrictOption,
  ): Promise<boolean> {
    return this._mainFrame._read(
      'locator.isChecked',
      'page.isChecked',
      selector,
      options,
    )
  }

  isDisabled(
    selector: string,
    options?: TimeoutOption & StrictOption,
  ): Promise<boolean> {
    return this._mainFrame._read(
      'locator.isDisabled',
      'page.isDisabled',
      selector,
      options,
    )
  }

  isEditable(
    selector: string,
    options?: TimeoutOption & StrictOption,
  ): Promise<boolean> {
    return this._mainFrame._read(
      'locator.isEditable',
      'page.isEditable',
      selector,
      options,
    )
  }

  isEnabled(
    selector: string,
    options?: TimeoutOption & StrictOption,
  ): Promise<boolean> {
    return this._mainFrame._read(
      'locator.isEnabled',
      'page.isEnabled',
      selector,
      options,
    )
  }

  isHidden(selector: string, options?: StrictOption): Promise<boolean> {
    return this._mainFrame._isVisible(
      'locator.isHidden',
      'page.isHidden',
      selector,
      options,
    )
  }

  isVisible(selector: string, options?: StrictOption): Promise<boolean> {
    return this._mainFrame._isVisible(
      'locator.isVisible',
      'page.isVisible',
      selector,
      options,
    )
  }

  $(selector: string, options?: StrictOption): Promise<ElementHandle | null> {
    return this._mainFrame._elementHandle(
      'page.$',
      selector,
      options?.strict === true,
      true,
    )
  }

  $$(selector: string): Promise<ElementHandle[]> {
    return this._mainFrame._elementHandles('page.$$', selector)
  }

  async $eval(
    selector: string,
    pageFunction: unknown,
    arg?: unknown,
  ): Promise<unknown> {
    // biome-ignore lint/complexity/noArguments: upstream counts the arguments passed, not the parameters.
    assertMaxArguments('page.$eval', arguments.length, 3)
    const api = 'page.$eval'
    return await this._mainFrame._evaluateOn(
      api,
      selector,
      false,
      pageCall(api, 'query', pageFunction, arg),
      true,
    )
  }

  async $$eval(
    selector: string,
    pageFunction: unknown,
    arg?: unknown,
  ): Promise<unknown> {
    // biome-ignore lint/complexity/noArguments: upstream counts the arguments passed, not the parameters.
    assertMaxArguments('page.$$eval', arguments.length, 3)
    const api = 'page.$$eval'
    return await this._mainFrame._evaluateAll(
      api,
      selector,
      pageCall(api, 'query', pageFunction, arg),
    )
  }

  waitForSelector(
    selector: string,
    options?: WaitForSelectorOptions,
  ): Promise<ElementHandle | null> {
    return this._mainFrame._waitForSelector(
      'page.waitForSelector',
      selector,
      options,
      false,
    )
  }

  // ---- the document -------------------------------------------------------------------

  goto(url: string, options?: NavigateOptions): Promise<Response | null> {
    return this._mainFrame._goto('page.goto', url, options)
  }

  reload(options?: NavigateOptions): Promise<Response | null> {
    return this.#history('page.reload', options)
  }

  goBack(options?: NavigateOptions): Promise<Response | null> {
    return this.#history('page.goBack', options)
  }

  goForward(options?: NavigateOptions): Promise<Response | null> {
    return this.#history('page.goForward', options)
  }

  waitForLoadState(state?: LoadState, options?: TimeoutOption): Promise<void> {
    return this._mainFrame._waitForLoadState(
      'page.waitForLoadState',
      state,
      options,
    )
  }

  waitForNavigation(
    options?: WaitForNavigationOptions,
  ): Promise<Response | null> {
    return this._waitForNavigation(
      'page.waitForNavigation',
      this._mainFrame,
      options,
    )
  }

  waitForURL(
    url: UrlMatcher,
    options?: TimeoutOption & { waitUntil?: LoadState },
  ): Promise<void> {
    return this._waitForURL('page.waitForURL', this._mainFrame, url, options)
  }

  async title(): Promise<string> {
    const api = 'page.title'
    const title = await invoke(
      'page.title',
      { api, timeoutMs: 0 },
      { page: await this._brand(api) },
    )
    // Reply facts own the cached title, so an older result cannot replace a newer read.
    return title
  }

  content(): Promise<string> {
    return this._mainFrame._content('page.content')
  }

  async emulateMedia(options: MediaOptions = {}): Promise<void> {
    const api = 'page.emulateMedia'
    const allowed = {
      media: ['screen', 'print', 'no-override'],
      colorScheme: ['dark', 'light', 'no-preference', 'no-override'],
      reducedMotion: ['reduce', 'no-preference', 'no-override'],
      forcedColors: ['active', 'none', 'no-override'],
      contrast: ['no-preference', 'more', 'no-override'],
    }
    const input: Partial<Record<keyof MediaOptions, string>> = {}
    for (const key of Object.keys(allowed) as (keyof MediaOptions)[]) {
      const value = options[key] === null ? 'no-override' : options[key]
      if (value === undefined) continue
      if (typeof value !== 'string' || !allowed[key].includes(value))
        throw clientError(
          api,
          `${key}: expected one of (${allowed[key].join('|')})`,
        )
      input[key] = value
    }
    await invoke('page.emulateMedia', this._mainFrame._call(api, 0), {
      page: await this._brand(api),
      ...input,
    })
  }

  addScriptTag(options: ScriptTagOptions = {}): Promise<ElementHandle> {
    return this._mainFrame._addTag(
      'page.addScriptTag',
      'page.addScriptTag',
      options,
    )
  }

  addStyleTag(options: StyleTagOptions = {}): Promise<ElementHandle> {
    return this._mainFrame._addTag(
      'page.addStyleTag',
      'page.addStyleTag',
      options,
    )
  }

  setContent(html: string, options?: NavigateOptions): Promise<void> {
    return this._mainFrame._setContent('page.setContent', html, options)
  }

  async evaluate(
    pageFunction: unknown,
    arg?: unknown,
    options?: EvaluateOptions,
  ): Promise<unknown> {
    // biome-ignore lint/complexity/noArguments: upstream counts the arguments passed, not the parameters.
    assertMaxArguments('page.evaluate', arguments.length, 3)
    return await this._mainFrame._evaluate(
      'page.evaluate',
      pageFunction,
      arg,
      options,
    )
  }

  waitForFunction(
    pageFunction: unknown,
    arg?: unknown,
    options?: WaitForFunctionOptions,
  ): Promise<WaitHandle> {
    return this._mainFrame._waitForFunction(
      'page.waitForFunction',
      pageFunction,
      arg,
      options,
    )
  }

  /**
   * upstream's wait is a server round trip, after which its frame tree is current; Neo's sleeps
   * in the realm, then reads the trees of the run's working pages (FR-1).
   */
  async waitForTimeout(timeout: number): Promise<void> {
    await sleep('page.waitForTimeout', timeout)
    await this._context._refreshWorkingFrames('page.waitForTimeout')
  }

  ariaSnapshot(options?: SnapshotOptions): Promise<string> {
    return this._mainFrame._ariaSnapshot('page.ariaSnapshot', null, options)
  }

  ariaSnapshotJSON(options?: SnapshotOptions): Promise<AriaSnapshotJSON> {
    return this._mainFrame._ariaSnapshotJSON(
      'page.ariaSnapshotJSON',
      null,
      options,
    )
  }

  screenshot(options?: ScreenshotOptionsInput): Promise<unknown> {
    return this._mainFrame._screenshot('page.screenshot', null, options)
  }

  /** upstream's client turns numeric sizes and margins into CSS pixel strings. */
  async pdf(
    options: Record<string, unknown> & { path?: string } = {},
  ): Promise<unknown> {
    const api = 'page.pdf'
    if (options.path !== undefined)
      throw unsupportedError(
        api,
        'Neo returns the PDF instead of writing a file; omit path',
      )
    const wire: Record<string, unknown> = pick(options, PDF_KEYS)
    for (const size of ['width', 'height'] as const)
      if (typeof wire[size] === 'number') wire[size] = `${wire[size]}px`
    if (typeof wire.margin === 'object' && wire.margin !== null) {
      const margin: Record<string, unknown> = { ...wire.margin }
      for (const side of ['top', 'right', 'bottom', 'left'])
        if (typeof margin[side] === 'number') margin[side] = `${margin[side]}px`
      wire.margin = margin
    }
    return await invoke(
      'page.pdf',
      { api, timeoutMs: 0 },
      { page: await this._brand(api), ...wire },
    )
  }

  async opener(): Promise<Page | null> {
    const api = 'page.opener'
    const target = await this._brand(api)
    const { opener } = await invoke(
      'page.opener',
      { api, timeoutMs: 0 },
      { page: target },
    )
    const page = this._context._pageFromResult(opener)
    return page && !page.isClosed() ? page : null
  }

  async close(options: CloseOptions = {}): Promise<void> {
    const api = 'page.close'
    if (this._closed || this._tab === null) {
      this.#close()
      return
    }
    // Outside the try: the catch reads the code the door gave the op, and a refused brand's
    // error has the door's fields only after the script next yields to Rust.
    const page = await this._brand(api)
    try {
      await invoke(
        'page.close',
        { api, timeoutMs: 0 },
        {
          page,
          ...(options.runBeforeUnload
            ? { runBeforeUnload: true as const }
            : {}),
        },
      )
    } catch (error) {
      // As upstream: closing a page that is already gone is not an error.
      if (
        !['target_closed', 'page_closed'].includes(
          String(Reflect.get(Object(error), 'code')),
        ) ||
        options.runBeforeUnload
      )
        throw error
    }
  }

  waitForEvent(
    event: string,
    optionsOrPredicate: unknown = {},
  ): Promise<unknown> {
    const since = Date.now()
    return this._context._events.wait(
      'page.waitForEvent',
      this,
      event,
      optionsOrPredicate,
      since,
      this._timeoutSettings,
    )
  }

  // ---- events -----------------------------------------------------------------------------

  protected override _checkEvent(event: string | symbol): void {
    checkReported('page.on', event, 'page')
  }

  protected override _listenersChanged(
    event: string | symbol,
    listening: boolean,
  ): void {
    if (isReported(event, 'page'))
      this._context._events.listen(this, event, listening)
  }

  // ---- internals ------------------------------------------------------------------------

  /**
   * The handle for an operation on this page. The run's unopened page carries `tab: null`,
   * which asks the door to open its default tab. Once bound, the handle carries the native
   * tab supplied by the host's facts, results or events.
   */
  async _brand(api: string): Promise<PageHandle> {
    if (this._tab === null) {
      if (this._closed)
        throw clientError(
          api,
          'Target page, context or browser has been closed',
        )
      return { $neo: 'page', tab: null }
    }
    this._working = true
    return { $neo: 'page', tab: this._tab }
  }

  /**
   * The handle of a page that has its tab, for the event ops, whose page the door does not
   * resolve. The run's unopened page gets its tab first from one op the door does resolve,
   * `page.frames`, as its first op of any kind would; the reply binds it.
   */
  async _openedBrand(api: string): Promise<PageBrand> {
    if (this._tab === null) await this._refreshFrames(api)
    const brand = await this._brand(api)
    if (brand.tab === null)
      throw new Error(`${api}: Neo opened no tab for the run's page`)
    return brand
  }

  /** An element handle from an op result, revived or raw; null when the op found none. */
  _handle(value: unknown): ElementHandle | null {
    if (value instanceof ElementHandle) return value
    const brand = elementBrand(value)
    return brand ? new ElementHandle(this, brand) : null
  }

  /** Runs a navigation op, counted in `_navigationsInFlight` from the call until it settles. */
  async _navigating<T>(work: () => Promise<T>): Promise<T> {
    this._navigationsInFlight++
    try {
      return await work()
    } finally {
      this._navigationsInFlight--
    }
  }

  /** Applies what a navigation or wait returned; the frame tree is fetched if it was not. */
  /** A script's supplemental facts must belong to this Page's current admitted binding. */
  _evaluated(frames: FrameInfo[] | undefined, at: Observation | null): void {
    if (this.#payloadCurrent(at) && Array.isArray(frames))
      this._applyFrames(frames)
  }

  async _navigated(
    api: string,
    frame: Frame,
    result: NavigationResult | null,
    at: Observation | null,
  ): Promise<Response | null> {
    if (this.#payloadCurrent(at)) {
      if (typeof result?.url === 'string') {
        if (frame === this._mainFrame) {
          this._url = result.url
          this.#urlKnown = true
        } else frame._url = result.url
      }
      if (Array.isArray(result?.frames)) this._applyFrames(result.frames)
      else await this._refreshFrames(api)
    } else if (!this._closed) {
      // Keep the original operation's return value even if a fresh read of the replacement
      // binding fails. The stale payload never becomes the replacement's cached state.
      await this._refreshFrames(api).catch(() => undefined)
    }
    return typeof result?.response?.status === 'number'
      ? new Response(frame, result.response)
      : null
  }

  /** Both the payload's admitted binding and this Page's newest fact must agree. */
  #payloadCurrent(at: Observation | null): boolean {
    return (
      !this._closed &&
      at !== null &&
      at.payloadCurrent === true &&
      at.seq === this.#observedSeq
    )
  }

  /**
   * The run's page tree, read once before the agent's first statement (FR-1), so a script that
   * starts on a tab with iframes finds them in `frames()`, as upstream's does. A page with no
   * tab yet reads nothing, so a run that never touches `page` opens no tab. A refused or failed
   * read, as under the owner's takeover, keeps the main frame alone and the run starts: the
   * script's own first op meets the same refusal (sdk-skeleton's A4).
   */
  async _readInitialFrames(): Promise<void> {
    if (this._tab === null) return
    await this._refreshFrames('page.frames').catch(() => undefined)
  }

  async _refreshFrames(api: string): Promise<void> {
    const { value: tree, observed } = await invokeObserved(
      'page.frames',
      { api, timeoutMs: 0 },
      { page: await this._brand(api) },
    )
    // A reply without a tree leaves the caches as they were rather than failing the
    // navigation that already succeeded. The main frame's entry refreshes `url()`.
    if (this.#payloadCurrent(observed) && Array.isArray(tree?.frames))
      this._applyFrames(tree.frames)
  }

  /**
   * Replaces the cached frame tree, keeping each child frame's object (scripts hold them) and
   * marking frames that left the tree detached. Parents are linked after every frame exists,
   * so the order Rust lists them in does not matter.
   */
  _applyFrames(infos: FrameInfo[]): void {
    const main = infos.find((info) => info.parentId === null)
    if (main) {
      this._url = main.url
      this.#urlKnown = true
      this._mainFrame._name = main.name
    }
    const present = new Map<string, FrameInfo>()
    for (const info of infos) {
      if (info.parentId === null) continue
      present.set(info.id, info)
      let frame = this.#childFrames.get(info.id)
      if (!frame) {
        frame = new Frame(this, info.id, this._mainFrame)
        this.#childFrames.set(info.id, frame)
      }
      frame._update(info)
    }
    for (const [id, frame] of this.#childFrames) {
      const info = present.get(id)
      if (!info) {
        frame._detached = true
        this.#childFrames.delete(id)
        continue
      }
      frame._parentFrame =
        info.parentId === main?.id
          ? this._mainFrame
          : (this.#childFrames.get(info.parentId ?? '') ?? this._mainFrame)
    }
  }

  _applyFact(
    fact: PageFact,
    at: Observation | null,
    approved: number | null = null,
  ): void {
    if (fact.tab !== undefined) this._tab = fact.tab
    if (at === null) {
      if (fact.url !== undefined && !this.#urlKnown) {
        this._url = fact.url
        this.#urlKnown = true
      }
      return
    }
    if (at.seq < this.#observedSeq) return
    this.#observedSeq = at.seq
    if (fact.url !== undefined) {
      this._url = fact.url
      this.#urlKnown = true
    }
    if (fact.title !== undefined) this._title = fact.title
    if (fact.closed === true) {
      this.#approvedClose =
        approved === null ? null : { seq: approved, observed: at.seq }
      if (approved === null) this.#close()
      return
    }
    // A bound init page or result brand can carry identity without a liveness fact.
    if (fact.closed !== false) return
    this.#approvedClose = null
    if (this._closed && at.events < this.#closeEvent) return
    this.#liveHead = at.events
    this._closed = false
  }

  _closeEvent(seq: number, at: Observation | null = null): void {
    // Concurrent waits can receive the same event in successive observations. Any of
    // them may consume that still-approved record before running its predicate. A later
    // live fact clears the approval, so an older wait cannot close a re-admitted page.
    if (this.#approvedClose?.seq === seq) {
      this.#closeEvent = Math.max(this.#closeEvent, seq)
      this.#approvedClose = null
      this.#close()
      return
    }
    if (at !== null && at.seq < this.#observedSeq) return
    if (this.#approvedClose !== null) {
      this.#closeEvent = Math.max(this.#closeEvent, seq)
      return
    }
    if (seq <= this.#closeEvent) return
    this.#closeEvent = seq
    if (seq <= this.#liveHead) return
    this.#close()
  }

  /** Finish only this reply's pending approval, including if a listener interrupted delivery. */
  _settleClose(at: Observation | null): void {
    if (at === null || this.#approvedClose?.observed !== at.seq) return
    this.#closeEvent = Math.max(this.#closeEvent, this.#approvedClose.seq)
    this.#approvedClose = null
    this.#close()
  }

  #close(): void {
    if (this._closed) return
    this._closed = true
    this.emit('close', this)
  }

  /**
   * `waitForURL`. Strings and RegExps match in the page with vendored `urlMatches`, where
   * `URL` exists. A predicate runs here, on a URL-like object, once for the current URL and
   * then after each change: each wait asks Rust for a URL other than the one just tested.
   */
  async _waitForURL(
    api: string,
    frame: Frame,
    url: UrlMatcher,
    options: TimeoutOption & SignalOption & { waitUntil?: LoadState } = {},
  ): Promise<void> {
    // Neo's waitForURL keeps the prefix it always gave; upstream's depends on whether the URL
    // already matched (waitForLoadState's check, or waitForNavigation's wrapped one).
    const waitUntil = knownLoadState(options.waitUntil ?? 'load')
    if (waitUntil === null) throw clientError(api, loadStateError('waitUntil'))
    const timeout = frame._navigationTimeout(options)
    const signal = signalOf(api, options)
    // upstream waits for a URL with a Waiter (waitForNavigation, or waitForLoadState once it
    // matched), whose abort rejects the whole wait; each op inside takes the signal too.
    await untilAborted(
      signal,
      () => abortedError(api, signal as SignalLike),
      () => this.#waitForURL(api, frame, url, waitUntil, timeout, signal),
    )
  }

  /** `_waitForURL`'s waits, once its options are checked. */
  async #waitForURL(
    api: string,
    frame: Frame,
    url: UrlMatcher,
    waitUntil: LoadState,
    timeout: number,
    signal: SignalLike | undefined,
  ): Promise<void> {
    const args = await frame._frameArgs(api)
    if (typeof url !== 'function') {
      const pattern =
        typeof url === 'string'
          ? { glob: url }
          : url instanceof RegExp
            ? { regexSource: url.source, regexFlags: url.flags }
            : undefined
      if (!pattern)
        throw clientError(api, 'url: expected a string, a RegExp or a function')
      const { value: result, observed } = await invokeObserved(
        'page.waitForURL',
        frame._call(api, timeout),
        {
          ...args,
          url: pattern,
          waitUntil,
        },
        undefined,
        signal,
      )
      await this._navigated(api, frame, result, observed)
      return
    }
    const started = Date.now()
    for (;;) {
      const current = frame.url()
      let matched: unknown
      try {
        matched = await url(URL.parse(current) ?? current)
      } catch (error) {
        throw prefixed(api, error)
      }
      const elapsed = Date.now() - started
      const remaining = timeout === 0 ? 0 : Math.max(1, timeout - elapsed)
      if (matched) {
        await frame._waitForLoadState(api, waitUntil, {
          timeout: remaining,
          signal,
        })
        return
      }
      const { value: result, observed } = await invokeObserved(
        'page.waitForURL',
        frame._call(api, remaining),
        {
          ...args,
          url: {
            regexSource: `^(?!${escapeRegExp(current)}$)`,
            regexFlags: '',
          },
          waitUntil: 'commit',
        },
        undefined,
        signal,
      )
      await this._navigated(api, frame, result, observed)
    }
  }

  /**
   * Keep a single call-time fence and journal cursor across predicate checks. A predicate
   * runs in the agent realm; Rust retains later events while it runs, including synchronous
   * pushState bursts. Unmatched URLs wait only for commit, never for a stalled load.
   */
  async _waitForNavigation(
    api: string,
    frame: Frame,
    options: WaitForNavigationOptions = {},
  ): Promise<Response | null> {
    const signal = signalOf(api, options)
    return await untilAborted(
      signal,
      () => abortedError(api, signal as SignalLike),
      () => this.#waitForNavigation(api, frame, options, signal),
    )
  }

  /** The journal cursor and deadline survive re-waits and share the call's abort signal. */
  async #waitForNavigation(
    api: string,
    frame: Frame,
    options: WaitForNavigationOptions,
    signal: SignalLike | undefined,
  ): Promise<Response | null> {
    const since = Date.now()
    let after = this.#liveHead
    const waitUntil = loadState(api, 'waitUntil', options.waitUntil ?? 'load')
    const timeout = frame._navigationTimeout(options)
    const matcher = options.url
    const predicate = typeof matcher === 'function' ? matcher : undefined
    const url =
      typeof matcher === 'string'
        ? { glob: matcher }
        : matcher instanceof RegExp
          ? { regexSource: matcher.source, regexFlags: matcher.flags }
          : undefined
    if (matcher !== undefined && !url && !predicate)
      throw clientError(api, 'url: expected a string, a RegExp or a function')
    const args = await frame._frameArgs(api)
    const deadline = new EventDeadline(api, null, since, timeout)
    const rethrow = (error: unknown): never => {
      throw deadline.named(error)
    }
    for (;;) {
      const { value: result, observed } = await deadline
        .race(
          invokeObserved(
            'page.waitForNavigation',
            frame._call(api, deadline.left()),
            {
              ...args,
              since,
              after,
              ...(url ? { url } : {}),
              waitUntil: predicate ? 'commit' : waitUntil,
            },
            undefined,
            signal,
          ),
        )
        .catch(rethrow)
      const response = await deadline.race(
        this._navigated(api, frame, result, observed),
      )
      if (!predicate) {
        deadline.left()
        return response
      }
      after = result.navigationSeq
      let matched: unknown
      try {
        matched = await deadline.race(
          predicate(URL.parse(result.url ?? '') ?? result.url),
        )
      } catch (error) {
        throw prefixed(api, error)
      }
      if (matched) {
        await deadline.race(
          frame
            ._waitForLoadState(api, waitUntil, {
              timeout: deadline.left(),
              signal,
            })
            .catch(rethrow),
        )
        deadline.left()
        return response
      }
    }
  }

  async #history(
    api: 'page.reload' | 'page.goBack' | 'page.goForward',
    options: NavigateOptions = {},
  ): Promise<Response | null> {
    const waitUntil = loadState(api, 'waitUntil', options.waitUntil ?? 'load')
    const signal = signalOf(api, options)
    return await this._navigating(async () => {
      const { value: result, observed } = await invokeObserved(
        api,
        this._mainFrame._call(api, this._mainFrame._navigationTimeout(options)),
        { page: await this._brand(api), waitUntil },
        undefined,
        signal,
      )
      return await this._navigated(api, this._mainFrame, result, observed)
    })
  }
}
