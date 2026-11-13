import { type CallSite, captureSite, INTERNAL_SITE } from './call-site'
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

import type { UrlPattern } from '../probe/protocol'
import { type BindingCallback, exposeOnPage } from './bindings'
import {
  invokeObserved,
  type Observation,
  type PageFact,
  sleep,
  type WorkerInfo,
} from './bridge'
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
  type Callbacks,
  type CallForm,
  type EvaluateOptions,
  exposesFunctions,
  pageCall,
  type ScriptParams,
} from './evaluation'
import { EventDeadline } from './event-deadline'
import { checkReported, isReported } from './events'
import {
  type AriaSnapshotOptions,
  type ClickOptions,
  type ContentOptions,
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
import { send } from './generated'
import { addInitScript, type Disposable } from './init-script'
import { Keyboard, Mouse } from './input'
import type { FrameLocator, Locator } from './locator'
import type { LocatorHandlerOptions } from './locator-handlers'
import { markHandle } from './options'
import { normalizePdfOptions, type PdfOptionsInput } from './pdf-options'
import { Publications } from './publication'
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
import { WebStorage } from './web-storage'
import type {
  DragOptions,
  FillOptions,
  FrameInfo,
  KeyOptions,
  LoadState,
  NavigationPublication,
  NavigationResult,
  PageBrand,
  PageHandle,
} from './wire'
import { Worker } from './worker'

/**
 * The pause before Neo reads once more a tab whose first read came back partial or empty: long
 * enough for an out-of-process frame's session to finish attaching, short enough to keep the
 * run's start quick.
 */
const FRAMES_RETRY_MS = 100

/** The read a navigation's reply still owes once its facts are applied (`Page._navigationReplied`). */
export type NavigationRead = 'none' | 'read' | 'reread'

/** Page-scoped preferences; omitted values are retained and null clears just that override. */
export type MediaOptions = {
  media?: 'screen' | 'print' | null
  colorScheme?: 'dark' | 'light' | 'no-preference' | null
  reducedMotion?: 'reduce' | 'no-preference' | null
  forcedColors?: 'active' | 'none' | null
  contrast?: 'no-preference' | 'more' | null
}

export type CloseOptions = { runBeforeUnload?: boolean; reason?: string }

export class Page extends EventEmitter {
  /** The browser's tab id, which the page's brand carries; null until a fact or result names it. */
  _tab: number | null
  #taskId: string | null = null
  readonly _context: BrowserContext
  readonly _timeoutSettings: TimeoutSettings
  readonly _mainFrame: Frame
  readonly keyboard: Keyboard
  readonly mouse: Mouse
  readonly localStorage: WebStorage
  readonly sessionStorage: WebStorage
  _url: string
  /** Historical opening events fill an unknown URL once. Live observations keep it current. */
  #urlKnown: boolean
  _title = ''
  _closed = false
  #viewportSize: { width: number; height: number } | null = null
  /** Upstream's client cache belongs to this Page once a call sets it, even if it rejects. */
  #viewportSet = false
  /** The newest host read applied to this tab. */
  #observedSeq = 0
  /** Frame removal fences include closed observations, independently of live-page admission. */
  #observedHead = 0
  /** Close events recorded through the last live observation belong to its earlier life. */
  #liveHead = 0
  /** A close can reach both a waiter and the event pump, including after re-admission. */
  #closeEvent = 0
  /** Host-approved close, deferred until that record's position in its own reply. */
  #approvedClose: { seq: number; observed: number } | null = null
  /**
   * Whether `waitForTimeout` reads this page's frame tree again (FR-1): the run's page, a page
   * the run opened or learned of after it started, and a tab the run's init listed once the
   * script ran an op on it. A listed tab the script never touched keeps the tree it had: Neo's
   * own worker-state subscription on every listed tab brands it with `working: false`
   * (`_brand`, `EventHub.sendListen`). Each refresh is an op on its tab, and a view that
   * follows the run's task goes to the tab of the task's latest op, so marking every listed tab
   * would send that view across all of them after each wait.
   */
  _working = false
  /**
   * Whether the script read this page's frames (`frames()`, `frame()`, a frame's
   * `childFrames()`), or Neo's first read of them failed or answered no tree. `waitForTimeout` then reads the
   * tree again, as for a working page, so a frame added later to a tab the script only watches
   * (another task's, say) shows after the script's next wait. Page facts carry no frame tree,
   * and an ordinary op's reply none either: only Neo's `page.frames` read, a navigation's or
   * load wait's reply, or an evaluate's reply update it.
   */
  _framesWatched = false
  /** The newest frame-tree version a reply gave this tab (`Envelope.frames`). */
  _framesChanged = 0
  /** The version the cached tree was read at; the tree is stale once `_framesChanged` passes it. */
  #framesReadAt = 0
  /** Whether the last read answered a partial tree, no tree, or failed. */
  #framesIncomplete = false
  /** Neo's read of this tab's tree in flight, which a settle step joins rather than repeats. */
  #framesRead: Promise<boolean> | null = null
  /** This tab's first read, its retry included, which a settle step waits for whole. */
  #firstRead: Promise<void> | null = null
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
  readonly #publications = new Publications()
  readonly #childFrames = new Map<string, Frame>()
  /** Snapshots may observe removal before the event reader delivers its retained record. */
  readonly #knownFrames = new Map<string, Frame>()
  /** Waiters and page/context listeners share the same event's Frame and apply it once. */
  readonly #frameEvents = new Map<number, Frame>()
  /** Per-frame journal watermarks prevent a late pull from undoing a newer event or snapshot. */
  readonly #frameSequences = new Map<string | null, number>()
  /** The bindings this page exposed (`exposeBinding`), which a page's calls are answered by. */
  readonly _exposed = new Map<string, BindingCallback>()

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
    this.localStorage = new WebStorage(this, 'local')
    this.sessionStorage = new WebStorage(this, 'session')
    markHandle(this)
  }

  // ---- synchronous state ----------------------------------------------------------------

  /** Neo addition: the browser tab id; null until the built-in page opens its tab. */
  get tabId(): number | null {
    return this._tab
  }

  /** Neo addition: the task that owns this tab, or null for a task-less tab. */
  get taskId(): string | null {
    return this.#taskId
  }

  context(): BrowserContext {
    return this._context
  }

  mainFrame(): Frame {
    return this._mainFrame
  }

  frames(): Frame[] {
    this._framesWatched = true
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
    return this._mainFrame.url()
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

  viewportSize(): { width: number; height: number } | null {
    return this.#viewportSize || null
  }

  async setViewportSize(viewportSize: {
    width: number
    height: number
  }): Promise<void> {
    const site = captureSite()
    // Playwright caches the caller's object before sending, without cloning it. Host facts
    // initialize a fresh realm; they must not overwrite a local call's synchronous cache.
    this.#viewportSize = viewportSize
    this.#viewportSet = true
    for (const side of ['width', 'height'] as const) {
      if (!Number.isInteger(viewportSize?.[side]))
        throw clientError(
          'page.setViewportSize',
          `viewportSize.${side}: expected integer`,
        )
    }
    const size = { width: viewportSize.width, height: viewportSize.height }
    await send['page.setViewportSize'](
      site,
      { api: 'page.setViewportSize', timeoutMs: 0 },
      { page: await this._brand('page.setViewportSize'), viewportSize: size },
    )
  }

  readonly #workers = new Map<string, Worker>()
  readonly #workerBatches = new Set<number>()
  #workerSnapshot: WorkerInfo[] | null = null

  workers(): Worker[] {
    return [...this.#workers.values()].filter((worker) => !worker._closed)
  }

  /** Facts, waits and listeners must return the same Worker, including after its close. */
  _worker(info: WorkerInfo): Worker {
    let worker = this.#workers.get(info.id)
    if (!worker) {
      worker = new Worker(this, info)
      this.#workers.set(info.id, worker)
    }
    return worker
  }

  /** A batch may carry console before close. Retire from facts only after all such replies
   * have delivered their records; newer foreground facts replace the pending snapshot. */
  _settleWorkerFacts(at: Observation | null): void {
    if (at) this.#workerBatches.delete(at.seq)
    if (this.#workerBatches.size === 0 && this.#workerSnapshot)
      this.#applyWorkers(this.#workerSnapshot)
  }

  #applyWorkers(workers: WorkerInfo[]): void {
    const live = new Set(workers.map((worker) => worker.id))
    for (const [id, worker] of this.#workers)
      if (!live.has(id)) worker._didClose()
    for (const info of workers) this._worker(info)
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
    const site = captureSite()
    // biome-ignore lint/complexity/noArguments: upstream counts the arguments passed, not the parameters.
    assertMaxArguments('page.$eval', arguments.length, 3)
    const api = 'page.$eval'
    return await this._mainFrame._evaluateOn(
      api,
      site,
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

  // ---- locator handlers (locator-handlers.ts) -------------------------------------------

  addLocatorHandler(
    locator: Locator,
    handler: (locator: Locator) => unknown,
    options?: LocatorHandlerOptions,
  ): Promise<void> {
    return this._context._locatorHandlers.add(
      'page.addLocatorHandler',
      this,
      locator,
      handler,
      options,
    )
  }

  removeLocatorHandler(locator: Locator): Promise<void> {
    return this._context._locatorHandlers.remove(
      'page.removeLocatorHandler',
      this,
      locator,
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
    const site = captureSite()
    return this._mainFrame._waitForLoadState(
      'page.waitForLoadState',
      site,
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
    const site = captureSite()
    return this._waitForURL(
      'page.waitForURL',
      site,
      this._mainFrame,
      url,
      options,
    )
  }

  /** The host admits/rechecks this page and collects in its root renderer. */
  async requestGC(): Promise<void> {
    const site = captureSite()
    const api = 'page.requestGC'
    await send['page.requestGC'](
      site,
      { api, timeoutMs: 0 },
      { page: await this._brand(api) },
    )
  }

  async title(): Promise<string> {
    const site = captureSite()
    const api = 'page.title'
    const title = await send['page.title'](
      site,
      { api, timeoutMs: 0 },
      { page: await this._brand(api) },
    )
    // Reply facts own the cached title, so an older result cannot replace a newer read.
    return title
  }

  content(options: ContentOptions = {}): Promise<string> {
    return this._mainFrame._content('page.content', options)
  }

  async emulateMedia(options: MediaOptions = {}): Promise<void> {
    const site = captureSite()
    const api = 'page.emulateMedia'
    const allowed = {
      media: ['screen', 'print', 'no-override'],
      colorScheme: ['dark', 'light', 'no-preference', 'no-override'],
      reducedMotion: ['reduce', 'no-preference', 'no-override'],
      forcedColors: ['active', 'none', 'no-override'],
      contrast: ['no-preference', 'more', 'no-override'],
    } as const
    const input: Partial<Record<keyof MediaOptions, string>> = {}
    for (const key of Object.keys(allowed) as (keyof MediaOptions)[]) {
      const value = options[key] === null ? 'no-override' : options[key]
      if (value === undefined) continue
      if (
        typeof value !== 'string' ||
        !allowed[key].some((candidate) => candidate === value)
      )
        throw clientError(
          api,
          `${key}: expected one of (${allowed[key].join('|')})`,
        )
      input[key] = value
    }
    await send['page.emulateMedia'](site, this._mainFrame._call(api, 0), {
      page: await this._brand(api),
      media: allowed.media.find((value) => value === input.media),
      colorScheme: allowed.colorScheme.find(
        (value) => value === input.colorScheme,
      ),
      reducedMotion: allowed.reducedMotion.find(
        (value) => value === input.reducedMotion,
      ),
      forcedColors: allowed.forcedColors.find(
        (value) => value === input.forcedColors,
      ),
      contrast: allowed.contrast.find((value) => value === input.contrast),
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

  /**
   * upstream's `page.addInitScript`: the script runs in the main world of every document this
   * page loads from now on, before the page's own scripts, until the run ends or the returned
   * disposable is disposed. The document already loaded does not run it.
   */
  addInitScript(
    script: unknown,
    arg?: unknown,
    options?: unknown,
  ): Promise<Disposable> {
    const site = captureSite()
    const api = 'page.addInitScript'
    return addInitScript(
      api,
      {
        register: async (source) =>
          await send['page.addInitScript'](
            site,
            { api, timeoutMs: 0 },
            { page: await this._brand(api), ...source },
          ),
        expose: (name, callback) =>
          exposeOnPage(this, site, api, name, callback, true),
      },
      script,
      arg,
      options,
    )
  }

  /**
   * upstream's `page.exposeBinding`: `window[name]` in every document of this page, the loaded
   * ones included, calls `callback({ context, page, frame }, ...args)` in the script and returns
   * its result, until the run ends or the returned disposable is disposed.
   */
  exposeBinding(name: string, callback: BindingCallback): Promise<Disposable> {
    const site = captureSite()
    return exposeOnPage(this, site, 'page.exposeBinding', name, callback)
  }

  /** upstream's `page.exposeFunction`: [`exposeBinding`] without the source argument. */
  exposeFunction(
    name: string,
    callback: (...args: unknown[]) => unknown,
  ): Promise<Disposable> {
    const site = captureSite()
    return exposeOnPage(
      this,
      site,
      'page.exposeFunction',
      name,
      (_source, ...args) => Reflect.apply(callback, undefined, args),
    )
  }

  /**
   * `pageCall` for an evaluate on this page. With `exposeFunctions`, each function in the
   * argument is exposed as this page's binding off `window` before the op that names it goes
   * out, as upstream's `serializeArgumentWithCallbacks` awaits `_exposeEvaluateCallback`. A
   * refused script fails in its op (after the wait, for a locator) and exposes nothing. The
   * callbacks last until the run ends, as every binding does; upstream also erases them when
   * the main frame loads a new document, which no script there can reach them from anyway.
   */
  async _evaluationScript(
    api: string,
    site: CallSite,
    form: CallForm,
    pageFunction: unknown,
    arg: unknown,
    options: unknown,
  ): Promise<ScriptParams> {
    const callbacks: Callbacks | undefined = exposesFunctions(options)
      ? []
      : undefined
    const script = pageCall(api, form, pageFunction, arg, options, callbacks)
    if (!callbacks?.length || script.refused !== undefined) return script
    const settled = await Promise.allSettled(
      callbacks.map(([name, callback]) =>
        exposeOnPage(
          this,
          site,
          api,
          name,
          (_source, ...args) => Reflect.apply(callback, undefined, args),
          true,
        ),
      ),
    )
    const failed = settled.find((outcome) => outcome.status === 'rejected')
    if (failed) {
      // A failure part way (a refused tab) leaves nothing behind, as `addInitScript`'s
      // `exposeFunctions` does: the evaluate never goes out, so no page can call these.
      await Promise.all(
        settled.map((outcome) =>
          outcome.status === 'fulfilled'
            ? outcome.value.dispose().catch(() => {})
            : undefined,
        ),
      )
      throw failed.reason
    }
    return script
  }

  /**
   * The frame a binding call came from, when the cache has it: the main frame for `null`, else
   * the child frame of that id. Synchronous, so the call's callback can run as the pump
   * dispatches it (`answerBindingCall`).
   */
  _knownBindingFrame(id: string | null): Frame | undefined {
    return id === null ? this._mainFrame : this.#childFrames.get(id)
  }

  /**
   * The frame a binding call came from: the main frame for `null`, else the child frame of that
   * id, the frame tree read again when the call names one the cache does not have yet. A frame
   * gone by then is answered as the main frame's.
   */
  async _bindingFrame(id: string | null): Promise<Frame> {
    if (id === null) return this._mainFrame
    const known = this.#childFrames.get(id)
    if (known) return known
    await this._refreshFrames('page.exposeBinding').catch(() => {})
    return this.#childFrames.get(id) ?? this._mainFrame
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

  ariaSnapshot(options?: AriaSnapshotOptions): Promise<string> {
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
    options: PdfOptionsInput & { path?: string } = {},
  ): Promise<unknown> {
    const site = captureSite()
    const api = 'page.pdf'
    if (options.path !== undefined)
      throw unsupportedError(
        api,
        'Neo returns the PDF instead of writing a file; omit path',
      )
    const wire = normalizePdfOptions(options)
    return await send['page.pdf'](
      site,
      { api, timeoutMs: 0 },
      {
        page: await this._brand(api),
        scale: wire.scale,
        displayHeaderFooter: wire.displayHeaderFooter,
        headerTemplate: wire.headerTemplate,
        footerTemplate: wire.footerTemplate,
        printBackground: wire.printBackground,
        landscape: wire.landscape,
        pageRanges: wire.pageRanges,
        format: wire.format,
        width: wire.width,
        height: wire.height,
        preferCSSPageSize: wire.preferCSSPageSize,
        margin: wire.margin,
        tagged: wire.tagged,
        outline: wire.outline,
      },
    )
  }

  async opener(): Promise<Page | null> {
    const site = captureSite()
    const api = 'page.opener'
    const target = await this._brand(api)
    const { opener } = await send['page.opener'](
      site,
      { api, timeoutMs: 0 },
      { page: target },
    )
    const page = this._context._pageFromResult(opener)
    return page && !page.isClosed() ? page : null
  }

  async close(options: CloseOptions = {}): Promise<void> {
    const site = captureSite()
    const api = 'page.close'
    if (this._closed || this._tab === null) {
      this.#close()
      return
    }
    // Outside the try: the catch reads the code the door gave the op, and a refused brand's
    // error has the door's fields only after the script next yields to Rust.
    const page = await this._brand(api)
    try {
      // Upstream selects a separate command: beforeunload can veto the close, so the door
      // leaves its closed-page facts to the tab watcher instead of declaring it closed now.
      const op = options.runBeforeUnload ? 'page.runBeforeUnload' : 'page.close'
      await send[op](site, { api, timeoutMs: 0 }, { page })
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
    const site = captureSite()
    const since = Date.now()
    return this._context._events.wait(
      'page.waitForEvent',
      site,
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
    const site = captureSite()
    if (isReported(event, 'page'))
      this._context._events.listen(site, this, event, listening)
  }

  // ---- internals ------------------------------------------------------------------------

  /**
   * The handle for an operation on this page. The run's unopened page carries `tab: null`,
   * which asks the door to open its default tab. Once bound, the handle carries the native
   * tab supplied by the host's facts, results or events. An op the script calls makes the
   * page working (FR-1, `_working`); Neo's own upkeep on a page passes `working: false`.
   */
  async _brand(
    api: string,
    { working = true }: { working?: boolean } = {},
  ): Promise<PageHandle> {
    if (this._tab === null) {
      if (this._closed)
        throw clientError(
          api,
          'Target page, context or browser has been closed',
        )
      return { $neo: 'page', tab: null }
    }
    if (working) this._working = true
    return { $neo: 'page', tab: this._tab }
  }

  /**
   * The handle of a page that has its tab, for the event ops, whose page the door does not
   * resolve. The run's unopened page gets its tab first from one op the door does resolve,
   * `page.frames`, as its first op of any kind would; the reply binds it. `options` as
   * `_brand`'s.
   */
  async _openedBrand(
    api: string,
    options: { working?: boolean } = {},
  ): Promise<PageBrand> {
    if (this._tab === null) await this._refreshFrames(api)
    const brand = await this._brand(api, options)
    if (brand.tab === null)
      throw new Error(`${api}: Neo opened no tab for the run's page`)
    // The generated input type has a nullable tab field; this check narrows it to the
    // output-side brand, whose tab is always open. The assertion changes no runtime value.
    return brand as PageBrand
  }

  /** An element handle from an op result, revived or raw; null when the op found none. */
  _handle(value: unknown): ElementHandle | null {
    if (value instanceof ElementHandle) return value
    const brand = elementBrand(value)
    return brand ? new ElementHandle(this, brand) : null
  }

  /** Runs a navigation op, counted in `_navigationsInFlight` from the call until it settles. */
  _navigating<T>(
    work: (ready: () => void, publication: NavigationPublication) => Promise<T>,
  ): Promise<T> {
    this._navigationsInFlight++
    return this.#publications.track(async (ready, publication) => {
      let outcome: { value: T } | { error: unknown }
      try {
        outcome = { value: await work(ready, publication) }
      } catch (error) {
        outcome = { error }
      } finally {
        this._navigationsInFlight--
      }
      // The settle steps skip a page while its navigation is in flight (Chromium answers no
      // command for it), so the navigating page itself is read here, once the counter dropped,
      // before the navigation, answered or failed, reaches the script. It stays inside the
      // tracked work on purpose: an evaluate that publishes this navigation (`publishReady`)
      // waits for it, so the navigation still resolves before that evaluate returns (upstream's
      // order, spec WA-104). The read is Neo's own and bounded (`LISTED_FRAMES_TIMEOUT_MS`).
      await this._context._settleFrames().catch(() => {})
      if ('error' in outcome) throw outcome.error
      return outcome.value
    })
  }

  /** Publish only a navigation whose requested load or native reply already completed. */
  async _publishCompleted(): Promise<void> {
    await this.#publications.publishReady()
  }

  /** A script's supplemental facts must belong to this Page's current admitted binding. */
  _evaluated(frames: FrameInfo[] | undefined, at: Observation | null): void {
    if (this.#payloadCurrent(at) && Array.isArray(frames))
      this._applyFrames(frames, at)
  }

  /** Applies native frame facts; older replies retain the fallback tree read. */
  /**
   * Applies the navigation facts an op answered: a navigation's result, a wait's reply, or a
   * wait that answers only the URL or nothing, so every field is optional here.
   */
  async _navigated(
    api: string,
    frame: Frame,
    result: Partial<NavigationResult> | null,
    at: Observation | null,
    ready?: () => void,
    replied: NavigationRead = this._navigationReplied(frame, result, at),
  ): Promise<Response | null> {
    if (replied === 'read') await this._refreshFrames(api)
    else if (replied === 'reread')
      // Keep the original operation's return value even if a fresh read of the replacement
      // binding fails. The stale payload never becomes the replacement's cached state.
      await this._refreshFrames(api).catch(() => undefined)
    // A modern native reply already attempted its bounded snapshot. If superseded, discard
    // those facts instead of reading a replacement renderer on an older op's completion.
    // After this mark a later evaluate may publish this navigation's public promise before
    // returning its own observation, so it waits for the rest of the tracked work. The only
    // browser await left is `_navigating`'s bounded settle read of the trees this navigation
    // left stale, which that evaluate joins so the navigation still resolves first.
    ready?.()
    // The navigation's frames were applied after its operation's settle step: a tree its reply
    // or its own read left stale is read once more before the navigation returns to the
    // script. Inside a tracked navigation (`ready` given) `_navigating` does that, once the
    // navigation is no longer in flight and its publication finished.
    if (!ready) await this._context._settleFrames().catch(() => {})
    return navigationResponse(frame, result)
  }

  /**
   * The facts a navigation's reply carries, applied to the cache: its URL, and its tree when its
   * bounded snapshot got one. A wait outside a tracked navigation applies them through
   * `invokeObserved`'s `apply`, before its settle step can read this tab again (bridge.ts); a
   * tracked one's tab is in flight, which the settle step leaves alone, so `_navigated` applies
   * them itself. Says which read is still owed: `read` when the reply attempted no snapshot,
   * `reread` when a newer fact replaced this reply's binding and it attempted none, else `none`.
   */
  _navigationReplied(
    frame: Frame,
    result: Partial<NavigationResult> | null,
    at: Observation | null,
  ): NavigationRead {
    if (this.#payloadCurrent(at)) {
      if (typeof result?.url === 'string') {
        if (frame === this._mainFrame) {
          this._url = result.url
          this.#urlKnown = true
        } else frame._url = result.url
      }
      // A supplemental tree is whole (the native side sends no partial one), but it does not
      // count as a read at the reply's version: that version was taken after the tree was, so
      // a frame that changed in between would be lost. A moved version is read once more.
      if (Array.isArray(result?.frames)) this._applyFrames(result.frames, at)
      else if (!result?.frameSnapshotAttempted) return 'read'
      // A bounded snapshot that got no tree (it ran out of time, or a navigation started):
      // the tree is read once the navigation ended (`_navigating`), or at the settle step.
      else this._framesUnknown()
      return 'none'
    }
    return !this._closed && !result?.frameSnapshotAttempted ? 'reread' : 'none'
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
   * This page's tree, read once before the agent's first statement (FR-1,
   * `BrowserContext._readInitialFrames`), so a script finds its iframes in `frames()` as upstream's
   * does. A page with no tab yet reads nothing, so a run that never touches `page` opens no tab.
   * A refused or failed read, as under the owner's takeover, keeps the main frame alone and the
   * run starts; the page is then read again at the script's next wait (`_framesWatched`).
   */
  _readInitialFrames(timeoutMs = 0): Promise<void> {
    if (this._tab === null || this._closed) return Promise.resolve()
    if (this.#firstRead) return this.#firstRead
    // A read that answers no tree or a partial one (an out-of-process frame's session was still
    // attaching, the tab was loading) is read once more before the tree is handed to the script.
    // One still not whole, or refused, stays marked (`#framesIncomplete`) and is read again
    // before the script's next operation settles.
    const first = (async () => {
      if (await this._readFrames(timeoutMs)) return
      if (this.#framesIncomplete && !this._closed) {
        await sleep('page.frames', FRAMES_RETRY_MS).catch(() => {})
        await this._readFrames(timeoutMs)
      }
    })().finally(() => {
      if (this.#firstRead === first) this.#firstRead = null
    })
    this.#firstRead = first
    return first
  }

  /**
   * This tab's part of a settle step (`BrowserContext._settleFrames`): its first read, retry and
   * all, if one is in flight, then one read if its tree is still stale.
   */
  async _settle(timeoutMs: number): Promise<void> {
    if (this.#firstRead) await this.#firstRead
    if (this._framesStale()) await this._readFrames(timeoutMs)
  }

  /**
   * Neo's read of this tab's tree, or the one already in flight: two settle steps, or a start
   * read and a settle step, never read one tab twice at once. Says whether the whole tree was
   * applied; a failed read marks the tree unknown and says no.
   */
  _readFrames(timeoutMs: number): Promise<boolean> {
    if (this.#framesRead) return this.#framesRead
    const read = this._refreshFrames('page.frames', timeoutMs)
      .catch(() => {
        this._framesUnknown()
        return false
      })
      .finally(() => {
        if (this.#framesRead === read) this.#framesRead = null
      })
    this.#framesRead = read
    return read
  }

  /**
   * One `page.frames` read of this tab. It is Neo's own (no call site, `working: false`): it makes
   * no page working, no tab the run's or its task's, and moves no view that follows the task
   * (neo-trace `NEO_READS`).
   */
  async _refreshFrames(api: string, timeoutMs = 0): Promise<boolean> {
    const site = INTERNAL_SITE
    // Taken before the read: a change of this tab's frames that the read races is newer than
    // this, and every reply carries the tab's version, so the next settle step reads again.
    const version = this._framesChanged
    const { value: tree, observed } = await invokeObserved(
      'page.frames',
      site,
      { api, timeoutMs },
      { page: await this._brand(api, { working: false }) },
    )
    // A reply without a tree leaves the caches as they were rather than failing the
    // navigation that already succeeded. The main frame's entry refreshes `url()`. A partial
    // tree (`incomplete`) is applied, as the frames it holds are real, and read again before the
    // script's next operation resolves. Says whether the whole tree was applied.
    if (this.#payloadCurrent(observed) && Array.isArray(tree?.frames)) {
      this._applyFrames(tree.frames, observed)
      this.#framesReadAt = version
      this.#framesIncomplete = tree.incomplete === true
      return !this.#framesIncomplete
    }
    // The host answers an object, `{}` when its bounded read got no tree; `null` is no answer at
    // all (a host without frame reads), which leaves nothing to read again.
    if (
      tree !== null &&
      typeof tree === 'object' &&
      this.#payloadCurrent(observed)
    )
      this._framesUnknown()
    return false
  }

  /**
   * The tree could not be read whole: a read failed, or answered no tree (the native read ran out
   * of its own time, or the tab began a navigation). It is read again before the script's next
   * operation resolves, and at its next wait.
   */
  _framesUnknown(): void {
    this.#framesIncomplete = true
    this._framesWatched = true
  }

  /** Records `version`, this tab's frame-tree version from a reply (`Envelope.frames`). */
  _frameVersion(version: number): void {
    this._framesChanged = Math.max(this._framesChanged, version)
  }

  /**
   * Whether a settle step must have this tab's tree read before the script's operation settles
   * (`BrowserContext._settleFrames`): a read of it is in flight (a page first seen mid-run, a
   * start read), its version moved since its tree was read, or its last read was not whole. A
   * tab with a navigation op in flight is left to that navigation's own reply.
   */
  _framesStale(): boolean {
    if (this._tab === null || this._closed || this._navigationsInFlight > 0)
      return false
    return (
      this.#framesRead !== null ||
      this.#firstRead !== null ||
      this.#framesIncomplete ||
      this._framesChanged > this.#framesReadAt
    )
  }

  /**
   * Replaces the cached frame tree, keeping each child frame's object (scripts hold them) and
   * marking frames that left the tree detached. Parents are linked after every frame exists,
   * so the order Rust lists them in does not matter.
   */
  _applyFrames(infos: FrameInfo[], at: Observation | null = null): void {
    const current = (id: string | null) =>
      !at || (this.#frameSequences.get(id) ?? 0) <= at.events
    const remember = (id: string | null) => {
      if (at)
        this.#frameSequences.set(
          id,
          Math.max(at.events, this.#frameSequences.get(id) ?? 0),
        )
    }
    const main = infos.find((info) => info.parentId === null)
    if (main && current(null)) {
      remember(null)
      this._url = main.url
      this.#urlKnown = true
      this._mainFrame._name = main.name
    }
    const present = new Map<string, FrameInfo>()
    for (const info of infos) {
      if (info.parentId === null || !current(info.id)) continue
      present.set(info.id, info)
      remember(info.id)
      let frame = this.#knownFrames.get(info.id)
      if (!frame) {
        frame = new Frame(this, info.id, this._mainFrame)
        this.#knownFrames.set(info.id, frame)
      }
      frame._detached = false
      this.#childFrames.set(info.id, frame)
      frame._update(info)
    }
    for (const [id, frame] of this.#childFrames) {
      if (!current(id)) continue
      const info = present.get(id)
      if (!info) {
        remember(id)
        frame._detached = true
        this.#childFrames.delete(id)
        continue
      }
      frame._parentFrame =
        info.parentId === main?.id
          ? this._mainFrame
          : (this.#childFrames.get(info.parentId ?? '') ?? this._mainFrame)
    }
    this.#pruneFrames()
  }

  /** Apply event-time identity before callbacks; a removed frame stays usable as metadata. */
  _eventFrame(
    seq: number,
    event: 'frameattached' | 'framenavigated' | 'framedetached',
    info: FrameInfo,
  ): Frame {
    const delivered = this.#frameEvents.get(seq)
    if (delivered) return delivered
    const id = info.parentId === null ? null : info.id
    const current = id === null ? this._mainFrame : this.#knownFrames.get(id)
    if (current && seq <= (this.#frameSequences.get(id) ?? 0)) {
      return this.#rememberFrameEvent(seq, current)
    }
    this.#frameSequences.set(id, seq)
    let frame: Frame
    if (info.parentId === null) {
      frame = this._mainFrame
      this._url = info.url
      this.#urlKnown = true
    } else {
      frame =
        this.#knownFrames.get(info.id) ??
        new Frame(this, info.id, this._mainFrame)
      this.#knownFrames.set(info.id, frame)
      frame._parentFrame =
        this.#knownFrames.get(info.parentId) ?? this._mainFrame
      if (event === 'framedetached') {
        frame._detached = true
        this.#childFrames.delete(info.id)
      } else {
        frame._detached = false
        this.#childFrames.set(info.id, frame)
      }
    }
    // An attach can arrive after a snapshot already read the initial document. Preserve
    // that document's URL; later navigation records still update the same Frame object.
    if (event !== 'frameattached' || frame.url() === '') frame._update(info)
    this.#pruneFrames()
    return this.#rememberFrameEvent(seq, frame)
  }

  /** Multiple readers share one event identity; the cache follows the journal's bound. */
  #rememberFrameEvent(seq: number, frame: Frame): Frame {
    this.#frameEvents.set(seq, frame)
    if (this.#frameEvents.size > 1024) {
      const oldest = this.#frameEvents.keys().next().value
      if (oldest !== undefined) this.#frameEvents.delete(oldest)
    }
    return frame
  }

  /** Retain at most a journal's worth of removed frames for delayed event delivery. */
  #pruneFrames(): void {
    let retained = this.#knownFrames.size - this.#childFrames.size
    for (const [id, frame] of this.#knownFrames) {
      if (retained <= 1024) break
      if (frame._detached) {
        this.#knownFrames.delete(id)
        this.#frameSequences.delete(id)
        retained--
      }
    }
  }

  /** A confirmed removal has no successful tree reply to retire the cached Frame objects. */
  #detachFrame(frame: Frame): void {
    const pending = [frame]
    for (const removed of pending) {
      pending.push(...removed.childFrames())
      if (removed._id === null) continue
      removed._detached = true
      this.#childFrames.delete(removed._id)
      this.#frameSequences.set(
        removed._id,
        Math.max(
          this.#observedHead,
          this.#frameSequences.get(removed._id) ?? 0,
        ),
      )
    }
    this.#pruneFrames()
  }

  /** A session swap can report Frame was detached too; only a fresh tree confirms removal. */
  async #frameWaitFailed(
    api: string,
    frame: Frame,
    error: unknown,
  ): Promise<void> {
    if (
      frame._id === null ||
      !(error instanceof Error) ||
      Reflect.get(error, 'code') !== 'target_closed'
    )
      return
    if (
      this._closed ||
      error.message.startsWith(`${api}: Navigating frame was detached!`)
    ) {
      this.#detachFrame(frame)
      return
    }
    if (!error.message.startsWith(`${api}: Frame was detached`)) return
    // Preserve the original rejection even if this optional read cannot finish. Its native
    // deadline prevents a failed wait from starting an unbounded renderer read during cleanup.
    await this._refreshFrames(api, 1_000).catch(() => undefined)
    if (this._closed) this.#detachFrame(frame)
  }

  /** Host refreshes preserve the cached object when dimensions agree, as upstream does. */
  #applyViewport(size: PageFact['viewportSize']): void {
    if (this.#viewportSet || size === undefined) return
    if (
      this.#viewportSize === null ||
      size === null ||
      this.#viewportSize.width !== size.width ||
      this.#viewportSize.height !== size.height
    )
      this.#viewportSize = size
  }

  _applyFact(
    fact: PageFact,
    at: Observation | null,
    approved: number | null = null,
  ): void {
    if (fact.tab !== undefined) this._tab = fact.tab
    if (fact.taskId !== undefined) this.#taskId = fact.taskId
    if (at === null) {
      this.#applyViewport(fact.viewportSize)
      if (fact.url !== undefined && !this.#urlKnown) {
        this._url = fact.url
        this.#urlKnown = true
      }
      return
    }
    if (at.seq < this.#observedSeq) return
    this.#observedSeq = at.seq
    if (fact.workers) {
      this.#workerSnapshot = fact.workers
      for (const worker of fact.workers) this._worker(worker)
      if (at.deferWorkers) this.#workerBatches.add(at.seq)
      else this._settleWorkerFacts(null)
    }
    this.#observedHead = Math.max(this.#observedHead, at.events)
    this.#applyViewport(fact.viewportSize)
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
    if (fact.closed !== false) {
      if (!this._closed && fact.workers)
        this._context._events.observeWorkers(this)
      return
    }
    this.#approvedClose = null
    if (this._closed && at.events < this.#closeEvent) return
    this.#liveHead = at.events
    this._closed = false
    if (fact.workers) this._context._events.observeWorkers(this)
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
    this._context._events.forgetWorkers(this)
    this.#workerSnapshot = []
    this._settleWorkerFacts(null)
    this.emit('close', this)
  }

  /**
   * `waitForURL`. Strings and RegExps match in the page with vendored `urlMatches`, where
   * `URL` exists. A predicate runs here, on a URL-like object, once for the current URL and
   * then after each change: each wait asks Rust for a URL other than the one just tested.
   */
  async _waitForURL(
    api: string,
    site: CallSite,
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
      () => this.#waitForURL(api, site, frame, url, waitUntil, timeout, signal),
    )
  }

  /** `_waitForURL`'s waits, once its options are checked. */
  async #waitForURL(
    api: string,
    site: CallSite,
    frame: Frame,
    url: UrlMatcher,
    waitUntil: LoadState,
    timeout: number,
    signal: SignalLike | undefined,
  ): Promise<void> {
    const args = await frame._frameArgs(api)
    const nativeWait = async <T>(work: () => Promise<T>): Promise<T> => {
      try {
        return await work()
      } catch (error) {
        await this.#frameWaitFailed(api, frame, error)
        throw error
      }
    }
    // Each wait's reply facts are applied before its settle step (`_navigationReplied`).
    const wait = (pattern: UrlPattern, until: LoadState, budget: number) =>
      nativeWait(async () => {
        let replied: NavigationRead | undefined
        const reply = await invokeObserved(
          'page.waitForURL',
          site,
          frame._call(api, budget),
          { ...args, url: pattern, waitUntil: until },
          undefined,
          signal,
          (value, at) => {
            replied = this._navigationReplied(frame, value, at)
          },
        )
        return { ...reply, replied }
      })

    if (typeof url !== 'function') {
      const pattern =
        typeof url === 'string'
          ? { glob: url }
          : url instanceof RegExp
            ? { regexSource: url.source, regexFlags: url.flags }
            : undefined
      if (!pattern)
        throw clientError(api, 'url: expected a string, a RegExp or a function')
      const {
        value: result,
        observed,
        replied,
      } = await wait(pattern, waitUntil, timeout)
      await this._navigated(api, frame, result, observed, undefined, replied)
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
        await nativeWait(() =>
          frame._waitForLoadState(api, site, waitUntil, {
            timeout: remaining,
            signal,
          }),
        )
        return
      }
      const {
        value: result,
        observed,
        replied,
      } = await wait(
        { regexSource: `^(?!${escapeRegExp(current)}$)`, regexFlags: '' },
        'commit',
        remaining,
      )
      await this._navigated(api, frame, result, observed, undefined, replied)
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
    const site = captureSite()
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
    // A wait op in flight races the deadline, as upstream's Waiter races its timer against the
    // event; the script's predicate too. Once the op that settles the call has answered, no
    // deadline check follows it (event-deadline.ts): the frame-cache refresh after a navigation
    // or a load-state wait is bounded by the deadline but never fails the call.
    for (;;) {
      let replied: NavigationRead | undefined
      const { value: result, observed } = await deadline
        .race(
          invokeObserved(
            'page.waitForNavigation',
            site,
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
            (value, at) => {
              replied = this._navigationReplied(frame, value, at)
            },
          ),
        )
        .catch(rethrow)
      const navigated = this._navigated(
        api,
        frame,
        result,
        observed,
        undefined,
        replied,
      )
      const settled = await deadline.within(navigated)
      const response = settled
        ? settled.value
        : navigationResponse(frame, result)
      if (!predicate) return response
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
        await frame
          ._waitForLoadState(
            api,
            site,
            waitUntil,
            { timeout: deadline.left(), signal },
            deadline,
          )
          .catch(rethrow)
        return response
      }
    }
  }

  async #history(
    api: 'page.reload' | 'page.goBack' | 'page.goForward',
    options: NavigateOptions = {},
  ): Promise<Response | null> {
    const site = captureSite()
    const waitUntil = loadState(api, 'waitUntil', options.waitUntil ?? 'load')
    const signal = signalOf(api, options)
    return await this._navigating(async (ready, publication) => {
      let completion: Promise<Response | null> | undefined
      const { value: result, observed } = await invokeObserved(
        api,
        site,
        this._mainFrame._call(api, this._mainFrame._navigationTimeout(options)),
        { page: await this._brand(api), waitUntil, publication },
        (result, observed) => {
          completion = this._navigated(
            api,
            this._mainFrame,
            result,
            observed,
            ready,
          )
        },
        signal,
      )
      return await (completion ??
        this._navigated(api, this._mainFrame, result, observed, ready))
    })
  }
}

/** The `Response` a navigation's result reports, or null for one with no HTTP response. */
function navigationResponse(
  frame: Frame,
  result: Partial<NavigationResult> | null,
): Response | null {
  return typeof result?.response?.status === 'number'
    ? new Response(frame, result.response)
    : null
}
