/**
 * Playwright's Page over one tab.
 *
 * Playwright's synchronous accessors (`url()`, `isClosed()`, `frames()`, `mainFrame()`) read
 * caches here. They are refreshed by the host's page facts after every op, by the URL and
 * frame tree each navigation or wait returns, and by events (a close, a popup). The run's
 * default page may not have a tab yet (a new task); its first op opens one, once, however
 * many ops race to be first.
 */
import { invoke, sleep } from './bridge'
import type { BrowserContext } from './context'
import { ElementHandle, elementBrand } from './element-handle'
import { EventEmitter } from './emitter'
import { clientError, prefixed, unsupportedError } from './errors'
import { checkReported, isReported } from './events'
import {
  type ClickOptions,
  type ExactOption,
  Frame,
  frameMatches,
  loadState,
  type NavigateOptions,
  type ScreenshotOptionsInput,
  type SnapshotOptions,
  type StrictOption,
  type TextMatch,
  type UrlMatcher,
  type ValueHandle,
  type WaitForFunctionOptions,
  type WaitForSelectorOptions,
} from './frame'
import { Keyboard, Mouse } from './input'
import type { FrameLocator, Locator } from './locator'
import { markHandle, pick } from './options'
import { Response } from './response'
import type { LocatorOptions } from './selectors'
import { type TimeoutOption, TimeoutSettings } from './timeouts'
import { urlLike } from './url'
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
  PageBrand,
  PageFact,
} from './wire'

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
  /** Core's page id; null until the run's lazy default page opens its tab. */
  _id: number | null
  /** The browser's tab id, which the page's brand carries; null until a fact or result names it. */
  _tab: number | null = null
  readonly _context: BrowserContext
  readonly _timeoutSettings: TimeoutSettings
  readonly _mainFrame: Frame
  readonly keyboard: Keyboard
  readonly mouse: Mouse
  _url: string
  _title = ''
  _closed = false
  #opening: Promise<number> | undefined
  readonly #childFrames = new Map<string, Frame>()

  constructor(context: BrowserContext, id: number | null, url = 'about:blank') {
    super()
    this._context = context
    this._id = id
    this._url = url
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
      throw new Error('Either name or url matcher should be specified')
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

  toJSON(): PageBrand | { $neo: 'page' } {
    return this._id === null ? { $neo: 'page' } : { $neo: 'page', id: this._id }
  }

  // ---- builders ---------------------------------------------------------------------------

  locator(selector: string, options?: LocatorOptions): Locator {
    return this._mainFrame.locator(selector, options)
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

  $eval(
    selector: string,
    pageFunction: unknown,
    arg?: unknown,
  ): Promise<unknown> {
    return this._mainFrame._evaluateOn(
      'page.$eval',
      selector,
      false,
      pageFunction,
      arg,
      true,
    )
  }

  $$eval(
    selector: string,
    pageFunction: unknown,
    arg?: unknown,
  ): Promise<unknown> {
    return this._mainFrame._evaluateAll(
      'page.$$eval',
      selector,
      pageFunction,
      arg,
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
    this._title = title
    return title
  }

  content(): Promise<string> {
    return this._mainFrame._content('page.content')
  }

  setContent(html: string, options?: NavigateOptions): Promise<void> {
    return this._mainFrame._setContent('page.setContent', html, options)
  }

  evaluate(pageFunction: unknown, arg?: unknown): Promise<unknown> {
    return this._mainFrame._evaluate('page.evaluate', pageFunction, arg)
  }

  waitForFunction(
    pageFunction: unknown,
    arg?: unknown,
    options?: WaitForFunctionOptions,
  ): Promise<ValueHandle> {
    return this._mainFrame._waitForFunction(
      'page.waitForFunction',
      pageFunction,
      arg,
      options,
    )
  }

  waitForTimeout(timeout: number): Promise<void> {
    return sleep('page.waitForTimeout', timeout)
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
    const { opener } = await invoke(
      'page.opener',
      { api, timeoutMs: 0 },
      { page: await this._brand(api) },
    )
    const page = this._context._pageFromResult(opener)
    return page && !page.isClosed() ? page : null
  }

  async close(options: CloseOptions = {}): Promise<void> {
    const api = 'page.close'
    if (this._id === null) {
      this._didClose()
      return
    }
    try {
      await invoke(
        'page.close',
        { api, timeoutMs: 0 },
        {
          page: await this._brand(api),
          ...(options.runBeforeUnload
            ? { runBeforeUnload: true as const }
            : {}),
        },
      )
    } catch (error) {
      // As upstream: closing a page that is already gone is not an error.
      if (
        Reflect.get(Object(error), 'code') !== 'target_closed' ||
        options.runBeforeUnload
      )
        throw error
    }
    if (!options.runBeforeUnload) this._didClose()
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

  /** Core's page id, opening the run's default tab on its first op. */
  async _pageId(api: string): Promise<number> {
    if (this._id !== null) return this._id
    if (this._closed)
      throw clientError(api, 'Target page, context or browser has been closed')
    this.#opening ??= this._context._openDefaultPage(api, this)
    return await this.#opening
  }

  /**
   * The page's handle for an op. Every fact, result and event the host sends names the tab;
   * a page whose tab nothing named yet is refused here rather than sent half-branded.
   */
  async _brand(api: string): Promise<PageBrand> {
    const id = await this._pageId(api)
    if (this._tab === null)
      throw clientError(
        api,
        'Neo has no tab id for this page; get the page again from context.pages()',
      )
    return { $neo: 'page', id, tab: this._tab }
  }

  /** An element handle from an op result, revived or raw; null when the op found none. */
  _handle(value: unknown): ElementHandle | null {
    if (value instanceof ElementHandle) return value
    const brand = elementBrand(value)
    return brand ? new ElementHandle(this, brand) : null
  }

  /** Applies what a navigation or wait returned; the frame tree is fetched if it was not. */
  async _navigated(
    api: string,
    frame: Frame,
    result: NavigationResult | null,
  ): Promise<Response | null> {
    if (typeof result?.url === 'string') {
      if (frame === this._mainFrame) this._url = result.url
      else frame._url = result.url
    }
    if (Array.isArray(result?.frames)) this._applyFrames(result.frames)
    else await this._refreshFrames(api)
    return typeof result?.response?.status === 'number'
      ? new Response(frame, result.response)
      : null
  }

  async _refreshFrames(api: string): Promise<void> {
    const tree = await invoke(
      'page.frames',
      { api, timeoutMs: 0 },
      { page: await this._brand(api) },
    )
    // A reply without a tree leaves the caches as they were rather than failing the
    // navigation that already succeeded. The main frame's entry refreshes `url()`.
    if (Array.isArray(tree?.frames)) this._applyFrames(tree.frames)
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

  _applyFact(fact: PageFact): void {
    if (fact.tab !== undefined) this._tab = fact.tab
    if (fact.url !== undefined) this._url = fact.url
    if (fact.title !== undefined) this._title = fact.title
    if (fact.closed) this._didClose()
  }

  _didClose(): void {
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
    options: TimeoutOption & { waitUntil?: LoadState } = {},
  ): Promise<void> {
    const waitUntil = loadState(api, 'waitUntil', options.waitUntil ?? 'load')
    const timeout = frame._navigationTimeout(options)
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
      const result = await invoke(
        'page.waitForURL',
        frame._call(api, timeout),
        {
          ...args,
          url: pattern,
          waitUntil,
        },
      )
      await this._navigated(api, frame, result)
      return
    }
    const started = Date.now()
    for (;;) {
      const current = frame.url()
      let matched: unknown
      try {
        matched = await url(urlLike(current) ?? current)
      } catch (error) {
        throw prefixed(api, error)
      }
      const elapsed = Date.now() - started
      const remaining = timeout === 0 ? 0 : Math.max(1, timeout - elapsed)
      if (matched) {
        await frame._waitForLoadState(api, waitUntil, { timeout: remaining })
        return
      }
      const result = await invoke(
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
      )
      await this._navigated(api, frame, result)
    }
  }

  async #history(
    api: 'page.reload' | 'page.goBack' | 'page.goForward',
    options: NavigateOptions = {},
  ): Promise<Response | null> {
    const waitUntil = loadState(api, 'waitUntil', options.waitUntil ?? 'load')
    const result = await invoke(
      api,
      this._mainFrame._call(api, this._mainFrame._navigationTimeout(options)),
      { page: await this._brand(api), waitUntil },
    )
    return await this._navigated(api, this._mainFrame, result)
  }
}
