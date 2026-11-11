/**
 * Playwright's Frame, and the one place a selector becomes an op.
 *
 * As in playwright-core, Page delegates to its main frame and Locator delegates to its frame
 * with `strict: true`. Each internal `_method(api, ...)` here is shared by all three, and `api`
 * is the method the script called, so `page.click` and `locator.click` send the same op with
 * their own name first in every message. Page and frame shortcuts take the first match unless
 * the script passes `{ strict: true }`. A child frame's wire id is the one Rust reported in the
 * page's frame tree; the main frame sends none.
 */

import { invoke, invokeObserved, sleep } from './bridge'
import { type CallSite, captureSite } from './call-site'
import type { ElementHandle } from './element-handle'
import { EventEmitter } from './emitter'
import {
  abortedError,
  clientError,
  plainError,
  unsupportedError,
} from './errors'
import {
  assertMaxArguments,
  type EvaluateOptions,
  fromPageValue,
  pageCall,
} from './evaluation'
import { EventDeadline } from './event-deadline'
import { dropFiles, inputFiles, withClientErrors } from './files'
import { OPTION_KEYS, send } from './generated'
import type { OpName } from './generated/ops'
import { FrameLocator, Locator, testIdAttributeName } from './locator'
import {
  FILL_KEYS,
  markHandle,
  PRESS_KEYS,
  pick,
  SCREENSHOT_KEYS,
  selectOptionValues,
  TYPE_KEYS,
} from './options'
import type { Page } from './page'
import type { Response } from './response'
import { saveScreenshot, screenshotType } from './screenshot'
import {
  type LocatorOptions,
  locatorBrand,
  withLocatorOptions,
} from './selectors'
import {
  type SignalLike,
  type SignalOption,
  signalOf,
  untilAborted,
} from './signal'
import { renderSnapshot } from './snapshot'
import { type TimeoutOption, timeoutMs } from './timeouts'
import type { KeyList } from './upstream-check'
import type { PickOptions } from './upstream-checks'
import { globMatches } from './url'
import {
  type AriaSnapshotJSON,
  type ByRoleOptions,
  getByAltTextSelector,
  getByLabelSelector,
  getByPlaceholderSelector,
  getByRoleSelector,
  getByTestIdSelector,
  getByTextSelector,
  getByTitleSelector,
  kAnyFrameSelector,
} from './vendored'
import type {
  CallHeader,
  DragOptions,
  FillOptions,
  FrameInfo,
  KeyOptions,
  LoadState,
  LocatorBrand,
  OpInputs,
  OpOutputs,
  PageHandle,
  PageScript,
  PointerOptions,
  ScreenshotOptions,
  SerializedValue,
  SnapshotResult,
  Strictness,
  TargetArgs,
  WaitForState,
} from './wire'

/** Tag inputs mirror Playwright; paths are read on Neo's host before entering the renderer. */
export type ScriptTagOptions = {
  url?: string
  path?: string
  content?: string
  type?: string
}
export type StyleTagOptions = Omit<ScriptTagOptions, 'type'>

export type StrictOption = { strict?: boolean }
export type ClickOptions = PointerOptions &
  TimeoutOption &
  StrictOption &
  SignalOption
export type TextMatch = string | RegExp
export type ExactOption = { exact?: boolean }
export type SnapshotOptions = TimeoutOption &
  SignalOption & {
    mode?: 'ai' | 'default'
    depth?: number
    boxes?: boolean
  }
export type NavigateOptions = TimeoutOption &
  SignalOption & {
    waitUntil?: LoadState | 'networkidle0'
    referer?: string
  }
export type WaitForSelectorOptions = TimeoutOption &
  StrictOption &
  SignalOption & { state?: WaitForState }
export type WaitForFunctionOptions = TimeoutOption &
  SignalOption & {
    polling?: number | 'raf'
  }
export type WaitForNavigationOptions = TimeoutOption &
  SignalOption & {
    url?: UrlMatcher
    waitUntil?: LoadState | 'networkidle0'
  }
/** One call's resolved timeout and the signal that aborts it (`Frame._wait`). */
export type Wait = { timeout: number; signal?: SignalLike }
export type UrlMatcher = string | RegExp | ((url: unknown) => unknown)

/** Ops that take a selector-built target and a strictness. */
export type SelectorOp = {
  [Op in OpName]: OpInputs[Op] extends { target: unknown; strict: Strictness }
    ? Op
    : never
}[OpName]

/** A load state upstream's verifyLoadState takes, `networkidle0` as `networkidle`, or null. */
export function knownLoadState(state: unknown): LoadState | null {
  if (state === 'networkidle0') return 'networkidle'
  if (
    state === 'load' ||
    state === 'domcontentloaded' ||
    state === 'networkidle' ||
    state === 'commit'
  )
    return state
  return null
}

/** upstream's verifyLoadState's message for the option `name`. */
export function loadStateError(name: string): string {
  return `${name}: expected one of (load|domcontentloaded|networkidle|commit)`
}

/**
 * upstream's verifyLoadState. Its client calls it before the API wrapper that prefixes a
 * failure with the method's name (PW:61763), so the message has no prefix: measured for
 * `setContent` and `waitForLoadState` with Playwright 1.64.1. The refusal is recorded under
 * `api`.
 */
export function loadState(
  api: string,
  name: string,
  state: unknown,
): LoadState {
  const known = knownLoadState(state)
  if (known === null) throw plainError(api, loadStateError(name))
  return known
}

/** The grouped reads share an identical target/strict input; each sender retains its result. */
type ElementReadOp =
  | 'locator.textContent'
  | 'locator.innerText'
  | 'locator.innerHTML'
  | 'locator.inputValue'
  | 'locator.isChecked'
  | 'locator.isDisabled'
  | 'locator.isEditable'
  | 'locator.isEnabled'

type EveryMatchReadOp =
  | 'locator.count'
  | 'locator.allTextContents'
  | 'locator.allInnerTexts'

export class Frame extends EventEmitter {
  readonly _page: Page
  /** Rust's frame id; null for the main frame, which the wire leaves out. */
  readonly _id: string | null
  _name = ''
  _url = ''
  _parentFrame: Frame | null
  _detached = false
  /** An event's synchronous callback sees its metadata without rewinding the live cache. */
  #eventView: { info: FrameInfo; detached: boolean } | null = null

  constructor(page: Page, id: string | null, parent: Frame | null) {
    super()
    this._page = page
    this._id = id
    this._parentFrame = parent
    markHandle(this)
  }

  page(): Page {
    return this._page
  }

  name(): string {
    return this.#eventView?.info.name ?? this._name
  }

  url(): string {
    return (
      this.#eventView?.info.url ??
      (this._id === null ? this._page._url : this._url)
    )
  }

  /**
   * Different readers can receive one journal record after a newer transition. Callbacks and
   * predicate invocation still read that record's fields on the canonical Frame. The view ends
   * before awaiting an async callback, whose continuation sees live state as in Playwright.
   */
  _duringEvent<T>(
    info: FrameInfo,
    event: 'frameattached' | 'framenavigated' | 'framedetached',
    read: () => T,
  ): T {
    const previous = this.#eventView
    this.#eventView = { info, detached: event === 'framedetached' }
    try {
      return read()
    } finally {
      this.#eventView = previous
    }
  }

  parentFrame(): Frame | null {
    return this._parentFrame
  }

  childFrames(): Frame[] {
    return this._page.frames().filter((frame) => frame._parentFrame === this)
  }

  isDetached(): boolean {
    return this.#eventView?.detached ?? this._detached
  }

  toJSON(): { name: string; url: string } {
    return { name: this.name(), url: this.url() }
  }

  // ---- builders: pure values ------------------------------------------------------------

  locator(selector: string, options?: LocatorOptions): Locator {
    return this._locator('frame.locator', selector, options)
  }

  /** `locator` for `api`, the method the script called: `page.locator` is its main frame's. */
  _locator(
    api: string,
    selector: string,
    options: LocatorOptions | undefined,
  ): Locator {
    return new Locator(this, withLocatorOptions(api, this, selector, options))
  }

  getByTestId(testId: TextMatch): Locator {
    return this.locator(getByTestIdSelector(testIdAttributeName(), testId))
  }

  getByAltText(text: TextMatch, options?: ExactOption): Locator {
    return this.locator(getByAltTextSelector(text, options))
  }

  getByLabel(text: TextMatch, options?: ExactOption): Locator {
    return this.locator(getByLabelSelector(text, options))
  }

  getByPlaceholder(text: TextMatch, options?: ExactOption): Locator {
    return this.locator(getByPlaceholderSelector(text, options))
  }

  getByText(text: TextMatch, options?: ExactOption): Locator {
    return this.locator(getByTextSelector(text, options))
  }

  getByTitle(text: TextMatch, options?: ExactOption): Locator {
    return this.locator(getByTitleSelector(text, options))
  }

  getByRole(role: string, options: ByRoleOptions = {}): Locator {
    return this.locator(getByRoleSelector(role, options))
  }

  frameLocator(selector?: string): FrameLocator {
    return new FrameLocator(
      this,
      selector === undefined ? kAnyFrameSelector : selector,
    )
  }

  // ---- shortcuts, each the locator op on the first match --------------------------------

  click(selector: string, options?: ClickOptions): Promise<void> {
    return this._click('frame.click', selector, options)
  }

  dblclick(selector: string, options?: ClickOptions): Promise<void> {
    return this._dblclick('frame.dblclick', selector, options)
  }

  hover(selector: string, options?: ClickOptions): Promise<void> {
    return this._hover('frame.hover', selector, options)
  }

  check(selector: string, options?: ClickOptions): Promise<void> {
    return this._check('frame.check', selector, true, options)
  }

  uncheck(selector: string, options?: ClickOptions): Promise<void> {
    return this._check('frame.uncheck', selector, false, options)
  }

  setChecked(
    selector: string,
    checked: boolean,
    options?: ClickOptions,
  ): Promise<void> {
    return this._check('frame.setChecked', selector, checked, options)
  }

  dragAndDrop(
    source: string,
    target: string,
    options?: DragOptions & TimeoutOption & StrictOption,
  ): Promise<void> {
    return this._dragAndDrop('frame.dragAndDrop', source, target, options)
  }

  fill(
    selector: string,
    value: string,
    options?: FillOptions & TimeoutOption & StrictOption,
  ): Promise<void> {
    return this._fill('frame.fill', selector, value, options)
  }

  press(
    selector: string,
    key: string,
    options?: KeyOptions & TimeoutOption & StrictOption,
  ): Promise<void> {
    return this._press('frame.press', selector, key, options)
  }

  type(
    selector: string,
    text: string,
    options?: KeyOptions & TimeoutOption & StrictOption,
  ): Promise<void> {
    return this._type('frame.type', selector, text, options)
  }

  focus(
    selector: string,
    options?: TimeoutOption & StrictOption,
  ): Promise<void> {
    return this._focus('frame.focus', selector, options)
  }

  selectOption(
    selector: string,
    values: unknown,
    options?: TimeoutOption & StrictOption & { force?: boolean },
  ): Promise<string[]> {
    return this._selectOption('frame.selectOption', selector, values, options)
  }

  setInputFiles(
    selector: string,
    files: unknown,
    options?: TimeoutOption & StrictOption,
  ): Promise<void> {
    return this._setInputFiles('frame.setInputFiles', selector, files, options)
  }

  dispatchEvent(
    selector: string,
    type: string,
    eventInit?: unknown,
    options?: TimeoutOption & StrictOption,
  ): Promise<void> {
    return this._dispatchEvent(
      'frame.dispatchEvent',
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
    return this._read(
      'locator.textContent',
      'frame.textContent',
      selector,
      options,
    )
  }

  innerText(
    selector: string,
    options?: TimeoutOption & StrictOption,
  ): Promise<string> {
    return this._read('locator.innerText', 'frame.innerText', selector, options)
  }

  innerHTML(
    selector: string,
    options?: TimeoutOption & StrictOption,
  ): Promise<string> {
    return this._read('locator.innerHTML', 'frame.innerHTML', selector, options)
  }

  getAttribute(
    selector: string,
    name: string,
    options?: TimeoutOption & StrictOption,
  ): Promise<string | null> {
    return this._getAttribute('frame.getAttribute', selector, name, options)
  }

  inputValue(
    selector: string,
    options?: TimeoutOption & StrictOption,
  ): Promise<string> {
    return this._read(
      'locator.inputValue',
      'frame.inputValue',
      selector,
      options,
    )
  }

  isChecked(
    selector: string,
    options?: TimeoutOption & StrictOption,
  ): Promise<boolean> {
    return this._read('locator.isChecked', 'frame.isChecked', selector, options)
  }

  isDisabled(
    selector: string,
    options?: TimeoutOption & StrictOption,
  ): Promise<boolean> {
    return this._read(
      'locator.isDisabled',
      'frame.isDisabled',
      selector,
      options,
    )
  }

  isEditable(
    selector: string,
    options?: TimeoutOption & StrictOption,
  ): Promise<boolean> {
    return this._read(
      'locator.isEditable',
      'frame.isEditable',
      selector,
      options,
    )
  }

  isEnabled(
    selector: string,
    options?: TimeoutOption & StrictOption,
  ): Promise<boolean> {
    return this._read('locator.isEnabled', 'frame.isEnabled', selector, options)
  }

  isHidden(selector: string, options?: StrictOption): Promise<boolean> {
    return this._isVisible(
      'locator.isHidden',
      'frame.isHidden',
      selector,
      options,
    )
  }

  isVisible(selector: string, options?: StrictOption): Promise<boolean> {
    return this._isVisible(
      'locator.isVisible',
      'frame.isVisible',
      selector,
      options,
    )
  }

  $(selector: string, options?: StrictOption): Promise<ElementHandle | null> {
    return this._elementHandle(
      'frame.$',
      selector,
      options?.strict === true,
      true,
    )
  }

  $$(selector: string): Promise<ElementHandle[]> {
    return this._elementHandles('frame.$$', selector)
  }

  async $eval(
    selector: string,
    pageFunction: unknown,
    arg?: unknown,
  ): Promise<unknown> {
    const site = captureSite()
    // biome-ignore lint/complexity/noArguments: upstream counts the arguments passed, not the parameters.
    assertMaxArguments('frame.$eval', arguments.length, 3)
    const api = 'frame.$eval'
    return await this._evaluateOn(
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
    assertMaxArguments('frame.$$eval', arguments.length, 3)
    const api = 'frame.$$eval'
    return await this._evaluateAll(
      api,
      selector,
      pageCall(api, 'query', pageFunction, arg),
    )
  }

  waitForSelector(
    selector: string,
    options?: WaitForSelectorOptions,
  ): Promise<ElementHandle | null> {
    return this._waitForSelector(
      'frame.waitForSelector',
      selector,
      options,
      false,
    )
  }

  // ---- the document -------------------------------------------------------------------

  goto(url: string, options?: NavigateOptions): Promise<Response | null> {
    return this._goto('frame.goto', url, options)
  }

  waitForLoadState(state?: LoadState, options?: TimeoutOption): Promise<void> {
    const site = captureSite()
    return this._waitForLoadState(
      'frame.waitForLoadState',
      site,
      state,
      options,
    )
  }

  waitForNavigation(
    options?: WaitForNavigationOptions,
  ): Promise<Response | null> {
    return this._page._waitForNavigation(
      'frame.waitForNavigation',
      this,
      options,
    )
  }

  waitForURL(
    url: UrlMatcher,
    options?: TimeoutOption & { waitUntil?: LoadState },
  ): Promise<void> {
    const site = captureSite()
    return this._page._waitForURL('frame.waitForURL', site, this, url, options)
  }

  async title(): Promise<string> {
    const site = captureSite()
    const api = 'frame.title'
    const { page, frame } = await this._frameArgs(api)
    return await send['page.title'](site, this._call(api, 0), { page, frame })
  }

  content(): Promise<string> {
    return this._content('frame.content')
  }

  addScriptTag(options: ScriptTagOptions = {}): Promise<ElementHandle> {
    return this._addTag('frame.addScriptTag', 'page.addScriptTag', options)
  }

  addStyleTag(options: StyleTagOptions = {}): Promise<ElementHandle> {
    return this._addTag('frame.addStyleTag', 'page.addStyleTag', options)
  }

  setContent(html: string, options?: NavigateOptions): Promise<void> {
    return this._setContent('frame.setContent', html, options)
  }

  async evaluate(
    pageFunction: unknown,
    arg?: unknown,
    options?: EvaluateOptions,
  ): Promise<unknown> {
    // biome-ignore lint/complexity/noArguments: upstream counts the arguments passed, not the parameters.
    assertMaxArguments('frame.evaluate', arguments.length, 3)
    return await this._evaluate('frame.evaluate', pageFunction, arg, options)
  }

  waitForFunction(
    pageFunction: unknown,
    arg?: unknown,
    options?: WaitForFunctionOptions,
  ): Promise<WaitHandle> {
    return this._waitForFunction(
      'frame.waitForFunction',
      pageFunction,
      arg,
      options,
    )
  }

  /** The same as `page.waitForTimeout`: a sleep, then the working pages' trees (FR-1). */
  async waitForTimeout(timeout: number): Promise<void> {
    await sleep('frame.waitForTimeout', timeout)
    await this._page._context._refreshWorkingFrames('frame.waitForTimeout')
  }

  // ---- internals shared with Page and Locator -----------------------------------------

  /** The header for one op, with a Playwright timeout already resolved. */
  _call(api: string, timeout: number): CallHeader {
    return { api, timeoutMs: timeoutMs(api, timeout) }
  }

  _timeout(options: TimeoutOption | undefined): number {
    return this._page._timeoutSettings.timeout(options)
  }

  /**
   * How long one call waits and what aborts it, as upstream's client `_timeout(options)` hands
   * both to each channel call: the resolved timeout and the call's own `signal`.
   */
  _wait(
    api: string,
    options: (TimeoutOption & SignalOption) | undefined,
  ): Wait {
    return { timeout: this._timeout(options), signal: signalOf(api, options) }
  }

  _navigationTimeout(options: TimeoutOption | undefined): number {
    return this._page._timeoutSettings.navigationTimeout(options)
  }

  async _frameArgs(api: string): Promise<{ page: PageHandle; frame?: string }> {
    const page = await this._page._brand(api)
    return this._id === null ? { page } : { page, frame: this._id }
  }

  async _locatorBrand(api: string, selector: string): Promise<LocatorBrand> {
    if (typeof selector !== 'string')
      throw clientError(
        api,
        `selector: expected string, got ${typeof selector}`,
      )
    return locatorBrand(api, await this._page._brand(api), this._id, selector)
  }

  /**
   * The addressing a locator op's input starts with: the locator's brand in this frame, and
   * whether a second match fails. A described op spreads it into its `send` literal, resolved
   * before the call header as `_selectorOp` does:
   * `const addressed = await this._targetArgs(api, selector, strict)` then
   * `send[op](site, this._call(api, wait.timeout), { ...addressed, ...fields }, wait.signal)`.
   * The op's own fields stay checked against Rust's input type; the spread ones are typed.
   */
  async _targetArgs(
    api: string,
    selector: string,
    strict: boolean | undefined,
  ): Promise<TargetArgs> {
    return {
      target: await this._locatorBrand(api, selector),
      strict: strict === true ? 'strict' : 'firstMatch',
    }
  }

  /**
   * One op on the element `selector` names in this frame, for the ops Rust does not describe
   * yet: their inputs are cast, so only the recorded wire examples check them.
   */
  async _selectorOp<Op extends SelectorOp>(
    op: Op,
    site: CallSite,
    api: string,
    selector: string,
    strict: boolean | undefined,
    wait: Wait,
    args: Omit<OpInputs[Op], 'target' | 'strict'>,
  ): Promise<OpOutputs[Op]> {
    const input = {
      ...(await this._targetArgs(api, selector, strict)),
      ...args,
    } as OpInputs[Op]
    return await invoke(
      op,
      site,
      this._call(api, wait.timeout),
      input,
      wait.signal,
    )
  }

  async _click(
    api: string,
    selector: string,
    options: ClickOptions = {},
  ): Promise<void> {
    const site = captureSite()
    const wait = this._wait(api, options)
    const deadline = new EventDeadline(api, null, Date.now(), wait.timeout)
    const strict = options.strict
    // Capture script options before resolving the page, which can await opening its tab.
    const picked = pick(options, OPTION_KEYS['locator.click'])
    const addressed = await this._targetArgs(api, selector, strict)
    const result = await send['locator.click'](
      site,
      this._call(api, wait.timeout),
      { ...addressed, options: picked },
      wait.signal,
    )
    if (result?.navigation) {
      try {
        await this._page._context._events.publishNavigation(
          this._page,
          result.navigation,
          api,
          deadline,
          wait.signal,
        )
      } catch (error) {
        // Input already landed. A caught publication failure must never invite replay.
        // Uncaught local errors remain script_error at the host boundary; RunReply derives
        // whole-run retryability from recorded writes, so that path is nonretryable too.
        if (error instanceof Error) {
          Object.assign(error, { delivery: 'unknown', retryable: false })
          if (Reflect.get(error, 'code') === 'wait_timeout')
            Object.assign(error, {
              code: 'navigation_observation_timeout',
              owner: 'neo',
            })
        }
        throw error
      }
    }
  }

  async _dblclick(
    api: string,
    selector: string,
    options: ClickOptions = {},
  ): Promise<void> {
    const site = captureSite()
    const strict = options.strict
    const wait = this._wait(api, options)
    const picked = pick(options, OPTION_KEYS['locator.dblclick'])
    const addressed = await this._targetArgs(api, selector, strict)
    await send['locator.dblclick'](
      site,
      this._call(api, wait.timeout),
      { ...addressed, options: picked },
      wait.signal,
    )
  }

  async _hover(
    api: string,
    selector: string,
    options: ClickOptions = {},
  ): Promise<void> {
    const site = captureSite()
    const strict = options.strict
    const wait = this._wait(api, options)
    const picked = pick(options, OPTION_KEYS['locator.hover'])
    const addressed = await this._targetArgs(api, selector, strict)
    await send['locator.hover'](
      site,
      this._call(api, wait.timeout),
      { ...addressed, options: picked },
      wait.signal,
    )
  }

  /** `check`, `uncheck` and `setChecked`, which upstream splits client-side the same way. */
  async _check(
    api: string,
    selector: string,
    checked: boolean,
    options: ClickOptions = {},
  ): Promise<void> {
    const site = captureSite()
    const op = checked ? 'locator.check' : 'locator.uncheck'
    const strict = options.strict
    const wait = this._wait(api, options)
    const picked = pick(options, OPTION_KEYS[op])
    const addressed = await this._targetArgs(api, selector, strict)
    await send[op](
      site,
      this._call(api, wait.timeout),
      { ...addressed, options: picked },
      wait.signal,
    )
  }

  async _dragAndDrop(
    api: string,
    source: string,
    target: string,
    options: DragOptions & TimeoutOption & StrictOption = {},
  ): Promise<void> {
    const site = captureSite()
    // Read before the first await, as every action reads its signal (signal.ts).
    const wait = this._wait(api, options)
    const to = await this._locatorBrand(api, target)
    const strict = options.strict
    const picked = pick(options, OPTION_KEYS['locator.dragTo'])
    const addressed = await this._targetArgs(api, source, strict)
    await send['locator.dragTo'](
      site,
      this._call(api, wait.timeout),
      { ...addressed, to, options: picked },
      wait.signal,
    )
  }

  async _fill(
    api: string,
    selector: string,
    value: string,
    options: FillOptions & TimeoutOption & StrictOption = {},
  ): Promise<void> {
    const site = captureSite()
    if (typeof value !== 'string')
      throw clientError(api, `value: expected string, got ${typeof value}`)
    await this._selectorOp(
      'locator.fill',
      site,
      api,
      selector,
      options.strict,
      this._wait(api, options),
      {
        value,
        options: pick(options, FILL_KEYS),
      },
    )
  }

  async _press(
    api: string,
    selector: string,
    key: string,
    options: KeyOptions & TimeoutOption & StrictOption = {},
  ): Promise<void> {
    const site = captureSite()
    await this._selectorOp(
      'locator.press',
      site,
      api,
      selector,
      options.strict,
      this._wait(api, options),
      {
        key,
        options: pick(options, PRESS_KEYS),
      },
    )
  }

  /** `type` and `pressSequentially`, one op. */
  async _type(
    api: string,
    selector: string,
    text: string,
    options: KeyOptions & TimeoutOption & StrictOption = {},
  ): Promise<void> {
    const site = captureSite()
    await this._selectorOp(
      'locator.pressSequentially',
      site,
      api,
      selector,
      options.strict,
      this._wait(api, options),
      { text, options: pick(options, TYPE_KEYS) },
    )
  }

  async _focus(
    api: string,
    selector: string,
    options: TimeoutOption & StrictOption = {},
  ): Promise<void> {
    const site = captureSite()
    await this._selectorOp(
      'locator.focus',
      site,
      api,
      selector,
      options.strict,
      this._wait(api, options),
      {},
    )
  }

  async _blur(
    api: string,
    selector: string,
    options: TimeoutOption & StrictOption = {},
  ): Promise<void> {
    const site = captureSite()
    await this._selectorOp(
      'locator.blur',
      site,
      api,
      selector,
      options.strict,
      this._wait(api, options),
      {},
    )
  }

  async _selectOption(
    api: string,
    selector: string,
    values: unknown,
    options: TimeoutOption & StrictOption & { force?: boolean } = {},
  ): Promise<string[]> {
    const site = captureSite()
    return await this._selectorOp(
      'locator.selectOption',
      site,
      api,
      selector,
      options.strict,
      this._wait(api, options),
      // biome-ignore format: Bun preserves literal layout in the byte-checked prelude asset.
      { ...selectOptionValues(api, values), ...pick(options, ['force'] satisfies KeyList<PickOptions['locator.selectOption']>) },
    )
  }

  async _selectText(
    api: string,
    selector: string,
    options: TimeoutOption & StrictOption & { force?: boolean } = {},
  ): Promise<void> {
    const site = captureSite()
    await this._selectorOp(
      'locator.selectText',
      site,
      api,
      selector,
      options.strict,
      this._wait(api, options),
      pick(options, ['force'] satisfies KeyList<
        PickOptions['locator.selectText']
      >),
    )
  }

  async _drop(
    selector: string,
    payload: { files?: unknown; data?: Record<string, string> },
    options: TimeoutOption &
      SignalOption & { position?: { x: number; y: number } },
  ): Promise<void> {
    const site = captureSite()
    const api = 'locator.drop'
    const files =
      payload.files === undefined ? undefined : dropFiles(api, payload.files)
    await withClientErrors(api, async () => {
      const wait = this._wait(api, options)
      const data = Object.entries(payload.data ?? {}).map(
        ([mimeType, value]) => ({
          mimeType,
          value,
        }),
      )
      const position = options.position
      const addressed = await this._targetArgs(api, selector, true)
      return await send['locator.drop'](
        site,
        this._call(api, wait.timeout),
        { ...addressed, files, data, position },
        wait.signal,
      )
    })
  }

  async _setInputFiles(
    api: string,
    selector: string,
    files: unknown,
    options: TimeoutOption & StrictOption = {},
  ): Promise<void> {
    const site = captureSite()
    // Upstream's client converts the files before it sends the selector anywhere.
    const input = inputFiles(api, files)
    await withClientErrors(api, () =>
      this._selectorOp(
        'locator.setInputFiles',
        site,
        api,
        selector,
        options.strict,
        this._wait(api, options),
        { files: input },
      ),
    )
  }

  async _dispatchEvent(
    api: string,
    selector: string,
    type: string,
    eventInit: unknown,
    options: TimeoutOption & StrictOption = {},
  ): Promise<void> {
    const site = captureSite()
    await this._selectorOp(
      'locator.dispatchEvent',
      site,
      api,
      selector,
      options.strict,
      this._wait(api, options),
      { type, eventInit: eventInit ?? {} },
    )
  }

  async _scrollIntoViewIfNeeded(
    api: string,
    selector: string,
    options: TimeoutOption & StrictOption = {},
  ): Promise<void> {
    const site = captureSite()
    await this._selectorOp(
      'locator.scrollIntoViewIfNeeded',
      site,
      api,
      selector,
      options.strict,
      this._wait(api, options),
      {},
    )
  }

  /** The reads that wait for one element, then return what the isolated world read. */
  _read<Op extends ElementReadOp>(
    op: Op,
    api: string,
    selector: string,
    options?: TimeoutOption & StrictOption,
  ): Promise<OpOutputs[Op]>
  async _read(
    op: ElementReadOp,
    api: string,
    selector: string,
    options: TimeoutOption & StrictOption = {},
  ): Promise<OpOutputs[ElementReadOp]> {
    const site = captureSite()
    const strict = options.strict
    const wait = this._wait(api, options)
    const addressed = await this._targetArgs(api, selector, strict)
    return await send[op](
      site,
      this._call(api, wait.timeout),
      { ...addressed },
      wait.signal,
    )
  }

  async _getAttribute(
    api: string,
    selector: string,
    name: string,
    options: TimeoutOption & StrictOption = {},
  ): Promise<string | null> {
    const site = captureSite()
    const strict = options.strict
    const wait = this._wait(api, options)
    const addressed = await this._targetArgs(api, selector, strict)
    return await send['locator.getAttribute'](
      site,
      this._call(api, wait.timeout),
      { ...addressed, name },
      wait.signal,
    )
  }

  /** `isVisible` and `isHidden` never wait, as upstream (their timeout is ignored). */
  async _isVisible(
    op: 'locator.isVisible' | 'locator.isHidden',
    api: string,
    selector: string,
    options: StrictOption = {},
  ): Promise<boolean> {
    const site = captureSite()
    const strict = options.strict
    const addressed = await this._targetArgs(api, selector, strict)
    return await send[op](site, this._call(api, 0), { ...addressed })
  }

  _everyMatch<Op extends EveryMatchReadOp>(
    op: Op,
    api: string,
    selector: string,
  ): Promise<OpOutputs[Op]>
  async _everyMatch(
    op: EveryMatchReadOp,
    api: string,
    selector: string,
  ): Promise<OpOutputs[EveryMatchReadOp]> {
    const site = captureSite()
    const target = await this._locatorBrand(api, selector)
    return await send[op](site, this._call(api, 0), { target })
  }

  /**
   * Upstream's `resolveSelector`: the selector Playwright's generator writes for the element
   * `selector` matches now, from the main frame down through every `<iframe>` on the way.
   */
  async _normalize(api: string, selector: string): Promise<string> {
    const site = captureSite()
    const target = await this._locatorBrand(api, selector)
    const resolved = await send['locator.normalize'](site, this._call(api, 0), {
      target,
      selector,
    })
    return resolved.selector
  }

  async _boundingBox(
    api: string,
    selector: string,
    options: TimeoutOption = {},
  ): Promise<OpOutputs['locator.boundingBox']> {
    const site = captureSite()
    const wait = this._wait(api, options)
    const addressed = await this._targetArgs(api, selector, true)
    return await send['locator.boundingBox'](
      site,
      this._call(api, wait.timeout),
      { ...addressed },
      wait.signal,
    )
  }

  async _waitForSelector(
    api: string,
    selector: string,
    options: WaitForSelectorOptions = {},
    omitReturnValue: boolean,
  ): Promise<ElementHandle | null> {
    const site = captureSite()
    const raw: unknown = options
    if (typeof raw === 'object' && raw !== null && 'visibility' in raw)
      throw clientError(
        api,
        'options.visibility is not supported, did you mean options.state?',
      )
    const state = options.state ?? 'visible'
    const strict = options.strict
    const wait = this._wait(api, options)
    const addressed = await this._targetArgs(api, selector, strict)
    const handle = await send['locator.waitFor'](
      site,
      this._call(api, wait.timeout),
      { ...addressed, state, omitReturnValue },
      wait.signal,
    )
    return this._page._handle(handle)
  }

  async _elementHandle(
    api: string,
    selector: string,
    strict: boolean,
    noWait: boolean,
    options: TimeoutOption = {},
  ): Promise<ElementHandle | null> {
    const site = captureSite()
    const wait = noWait ? { timeout: 0 } : this._wait(api, options)
    const addressed = await this._targetArgs(api, selector, strict)
    // Preserve the omitted field on waiting calls: noWait:false was never sent.
    const handle = noWait
      ? await send['locator.elementHandle'](
          site,
          this._call(api, wait.timeout),
          { ...addressed, noWait: true },
          wait.signal,
        )
      : await send['locator.elementHandle'](
          site,
          this._call(api, wait.timeout),
          { ...addressed },
          wait.signal,
        )
    return this._page._handle(handle)
  }

  async _elementHandles(
    api: string,
    selector: string,
  ): Promise<ElementHandle[]> {
    const site = captureSite()
    const target = await this._locatorBrand(api, selector)
    const handles = await send['locator.elementHandles'](
      site,
      this._call(api, 0),
      {
        target,
      },
    )
    return handles.flatMap((handle) => {
      const element = this._page._handle(handle)
      return element ? [element] : []
    })
  }

  /**
   * `fn(element, arg)` on `selector`'s element: `$eval` takes the first match now (and quotes
   * the selector as written when there is none), a locator waits for its element first.
   */
  async _evaluateOn(
    api: string,
    site: CallSite,
    selector: string,
    strict: boolean,
    script: PageScript,
    noWait: boolean,
    options: TimeoutOption & SignalOption = {},
  ): Promise<unknown> {
    const value = await this._selectorOp(
      'locator.evaluate',
      site,
      api,
      selector,
      strict,
      noWait ? { timeout: 0 } : this._wait(api, options),
      noWait ? { script, noWait: true, selector } : { script },
    )
    return fromPageValue(api, value)
  }

  /** `fn(elements, arg)` on every match of `selector` now. */
  async _evaluateAll(
    api: string,
    selector: string,
    script: PageScript,
  ): Promise<unknown> {
    const site = captureSite()
    const target = await this._locatorBrand(api, selector)
    const value = await invoke(
      'locator.evaluateAll',
      site,
      this._call(api, 0),
      {
        target,
        script,
      },
    )
    return fromPageValue(api, value)
  }

  /** A snapshot rooted at `selector`, or at the page when `selector` is null. */
  async _snapshot(
    api: string,
    selector: string | null,
    options: SnapshotOptions = {},
  ): Promise<SnapshotResult> {
    const site = captureSite()
    const mode = options.mode ?? 'default'
    // Capture options before resolving a locator's page, which can await opening its tab.
    const { depth, boxes } = options
    const wait = this._wait(api, options)
    if (selector !== null) {
      const addressed = await this._targetArgs(api, selector, true)
      return await send['locator.ariaSnapshot'](
        site,
        this._call(api, wait.timeout),
        { ...addressed, mode, depth, boxes },
        wait.signal,
      )
    }
    if (this._id !== null)
      throw unsupportedError(
        api,
        "snapshot a child frame through its iframe: page.locator('iframe').contentFrame().locator('body').ariaSnapshot()",
      )
    return await send['page.ariaSnapshot'](
      site,
      this._call(api, wait.timeout),
      {
        page: await this._page._brand(api),
        mode,
        depth,
        boxes,
      },
      wait.signal,
    )
  }

  async _ariaSnapshot(
    api: string,
    selector: string | null,
    options?: SnapshotOptions,
  ): Promise<string> {
    return renderSnapshot(await this._snapshot(api, selector, options))
  }

  async _ariaSnapshotJSON(
    api: string,
    selector: string | null,
    options?: SnapshotOptions,
  ): Promise<AriaSnapshotJSON> {
    return (await this._snapshot(api, selector, options)).json
  }

  async _screenshot(
    api: string,
    selector: string | null,
    options: ScreenshotOptionsInput = {},
  ): Promise<unknown> {
    const site = captureSite()
    const wire = await screenshotOptions(api, options)
    if (!wire.type && options.path)
      wire.type = screenshotType({ api, path: options.path })
    const { timeout, signal } = this._wait(api, options)
    const page = await this._page._brand(api)
    const input: OpInputs['page.screenshot'] =
      selector === null
        ? { page, ...wire }
        : {
            page,
            target: await this._locatorBrand(api, selector),
            strict: 'strict',
            ...wire,
          }
    const image = await invoke(
      'page.screenshot',
      site,
      this._call(api, timeout),
      input,
      signal,
    )
    await saveScreenshot({
      api,
      site,
      bytes: image,
      path: options.path,
      signal,
    })
    return image
  }

  async _goto(
    api: string,
    url: string,
    options: NavigateOptions = {},
  ): Promise<Response | null> {
    const site = captureSite()
    if (typeof url !== 'string')
      throw clientError(api, `url: expected string, got ${typeof url}`)
    const waitUntil = loadState(api, 'waitUntil', options.waitUntil ?? 'load')
    const signal = signalOf(api, options)
    return await this._page._navigating(async (ready, publication) => {
      const args = await this._frameArgs(api)
      let completion: Promise<Response | null> | undefined
      const { value: result, observed } = await invokeObserved(
        'page.goto',
        site,
        this._call(api, this._navigationTimeout(options)),
        {
          ...args,
          url,
          waitUntil,
          publication,
          ...pick(options, ['referer'] satisfies KeyList<
            PickOptions['page.goto']
          >),
        },
        (result, observed) => {
          // Decode-time publication precedes a later evaluate's observation. A stale
          // binding may still need browser work; _navigated marks readiness only after it.
          completion = this._page._navigated(api, this, result, observed, ready)
        },
        signal,
      )
      return await (completion ??
        this._page._navigated(api, this, result, observed, ready))
    })
  }

  async _waitForLoadState(
    api: string,
    site: CallSite,
    state: LoadState | undefined,
    options: TimeoutOption & SignalOption = {},
  ): Promise<void> {
    const verified = loadState(api, 'state', state ?? 'load')
    // upstream waits for a load state with a Waiter, whose abort rejects the whole wait.
    const signal = signalOf(api, options)
    await untilAborted(
      signal,
      () => abortedError(api, signal as SignalLike),
      async () => {
        const { value: result, observed } = await invokeObserved(
          'page.waitForLoadState',
          site,
          this._call(api, this._navigationTimeout(options)),
          { ...(await this._frameArgs(api)), state: verified },
          undefined,
          signal,
        )
        await this._page._navigated(api, this, result, observed)
      },
    )
  }

  async _content(api: string): Promise<string> {
    const site = captureSite()
    const { page, frame } = await this._frameArgs(api)
    return await send['page.content'](site, this._call(api, 0), { page, frame })
  }

  /** Page delegates here so frame routing, errors and returned node ownership stay identical. */
  async _addTag(
    api: string,
    op: 'page.addScriptTag' | 'page.addStyleTag',
    options: ScriptTagOptions,
  ): Promise<ElementHandle> {
    const site = captureSite()
    const keys =
      op === 'page.addScriptTag'
        ? (['url', 'path', 'content', 'type'] as const satisfies KeyList<
            PickOptions['page.addScriptTag']
          >)
        : (['url', 'path', 'content'] as const satisfies KeyList<
            PickOptions['page.addStyleTag']
          >)
    const input = pick(options, keys)
    for (const key of keys) {
      const value = input[key]
      if (value !== undefined && typeof value !== 'string')
        throw clientError(
          api,
          `${key}: expected string, got ${value === null ? 'object' : typeof value}`,
        )
    }
    let handle: ElementHandle | null
    try {
      const { value: result, observed } = await invokeObserved(
        op,
        site,
        this._call(api, 0),
        {
          ...(await this._frameArgs(api)),
          ...input,
        },
      )
      this._page._evaluated(result.frames, observed)
      handle = this._page._handle(result.element)
    } catch (error) {
      // File reads are upstream client work, whose errors carry no API prefix.
      if (
        error instanceof Error &&
        error.message.startsWith(`${api}: `) &&
        /^(?:E[A-Z0-9]+|UNKNOWN): .*?, (?:open|read) '/.test(
          error.message.slice(api.length + 2),
        )
      )
        error.message = error.message.slice(api.length + 2)
      throw error
    }
    if (!handle)
      throw plainError(api, 'The inserted tag did not return an element')
    return handle
  }

  async _setContent(
    api: string,
    html: string,
    options: NavigateOptions = {},
  ): Promise<void> {
    const site = captureSite()
    const waitUntil = loadState(api, 'waitUntil', options.waitUntil ?? 'load')
    const signal = signalOf(api, options)
    const { value: result, observed } = await invokeObserved(
      'page.setContent',
      site,
      this._call(api, this._navigationTimeout(options)),
      { ...(await this._frameArgs(api)), html, waitUntil },
      undefined,
      signal,
    )
    await this._page._navigated(api, this, result, observed)
  }

  /** upstream's `Frame.evaluate` after its argument count: the client's checks, then the op. */
  async _evaluate(
    api: string,
    pageFunction: unknown,
    arg: unknown,
    options: EvaluateOptions | undefined,
  ): Promise<unknown> {
    const site = captureSite()
    const script = await this._page._evaluationScript(
      api,
      site,
      'frame',
      pageFunction,
      arg,
      options,
    )
    const { value: result, observed } = await invokeObserved(
      'page.evaluate',
      site,
      this._call(api, 0),
      {
        ...(await this._frameArgs(api)),
        script,
      },
    )
    this._page._evaluated(result.frames, observed)
    await this._page._publishCompleted()
    return fromPageValue(api, result.value)
  }

  /** upstream's `Frame.waitForFunction`: its polling check, unprefixed, comes first. */
  async _waitForFunction(
    api: string,
    pageFunction: unknown,
    arg: unknown,
    options: WaitForFunctionOptions = {},
  ): Promise<WaitHandle> {
    const site = captureSite()
    if (typeof options.polling === 'string' && options.polling !== 'raf')
      throw plainError(api, `Unknown polling option: ${options.polling}`)
    const script = pageCall(api, 'frame', pageFunction, arg)
    const wait = this._wait(api, options)
    const { value, preview } = await invoke(
      'page.waitForFunction',
      site,
      this._call(api, wait.timeout),
      {
        ...(await this._frameArgs(api)),
        script,
        ...(typeof options.polling === 'number'
          ? { polling: options.polling }
          : {}),
      },
      wait.signal,
    )
    return new WaitHandle(value, preview)
  }

  /** Refreshes this frame from a frame-tree entry. */
  _update(info: FrameInfo): void {
    this._name = info.name
    this._url = info.url
  }
}

/** `page.frame(selector)`: a frame by name, or by its URL as a glob, RegExp or predicate. */
export function frameMatches(
  frame: Frame,
  selector: string | { name?: string; url?: UrlMatcher },
): boolean {
  const name = typeof selector === 'string' ? selector : selector.name
  if (name) return frame.name() === name
  const url = typeof selector === 'string' ? undefined : selector.url
  if (url === undefined) return false
  if (typeof url === 'string') return globMatches(url, frame.url())
  if (url instanceof RegExp) {
    url.lastIndex = 0
    return url.test(frame.url())
  }
  return Boolean(url(frame.url()))
}

export type ScreenshotOptionsInput = TimeoutOption &
  SignalOption &
  Omit<ScreenshotOptions, 'mask'> & {
    path?: string
    mask?: Locator[]
    maskColor?: string
  }

/** Locator instances stay in the script realm; only parsed locator brands cross to Rust. */
export async function screenshotOptions(
  api: string,
  options: ScreenshotOptionsInput,
): Promise<ScreenshotOptions> {
  const wire: ScreenshotOptions = pick(options, SCREENSHOT_KEYS)
  if (options.mask !== undefined) {
    wire.mask = await Promise.all(
      options.mask.map((locator) => {
        if (!(locator instanceof Locator))
          throw clientError(api, 'mask: expected a Locator')
        return locator._frame._locatorBrand(api, locator._selector)
      }),
    )
  }
  return wire
}

const HELD_VALUE =
  "Neo's waitForFunction handle holds the value, not the page's object; read it with jsonValue()"

/**
 * What `waitForFunction` resolves to: upstream's JSHandle for the truthy value
 * (`JSHandle.from`, PW:62147), as a snapshot. Neo keeps no page object past an op, so the
 * handle holds the value Rust read when the wait ended, in Playwright's format, and
 * upstream's preview of it. `jsonValue()` reads that value afresh on every call, as upstream's
 * does, and `toString()` is the preview. What needs the page's own object is refused.
 */
export class WaitHandle {
  readonly #value: SerializedValue
  readonly #preview: string

  constructor(value: SerializedValue, preview: string) {
    this.#value = value
    this.#preview = preview
    markHandle(this)
  }

  async jsonValue(): Promise<unknown> {
    return fromPageValue('jsHandle.jsonValue', this.#value)
  }

  /** The value, for a script that passes this handle as its argument to `api`. */
  _value(api: string): unknown {
    return fromPageValue(api, this.#value)
  }

  asElement(): null {
    return null
  }

  async dispose(): Promise<void> {}

  toString(): string {
    return this.#preview
  }

  evaluate(): Promise<never> {
    return Promise.reject(unsupportedError('jsHandle.evaluate', HELD_VALUE))
  }

  evaluateHandle(): Promise<never> {
    return Promise.reject(
      unsupportedError('jsHandle.evaluateHandle', HELD_VALUE),
    )
  }

  getProperty(): Promise<never> {
    return Promise.reject(unsupportedError('jsHandle.getProperty', HELD_VALUE))
  }

  getProperties(): Promise<never> {
    return Promise.reject(
      unsupportedError('jsHandle.getProperties', HELD_VALUE),
    )
  }
}
