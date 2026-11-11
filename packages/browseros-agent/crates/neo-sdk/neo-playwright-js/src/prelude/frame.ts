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
import { inputFiles, withClientErrors } from './files'
import { FrameLocator, Locator, testIdAttributeName } from './locator'
import type { OpName } from './ops.generated'
import {
  CHECK_KEYS,
  DBLCLICK_KEYS,
  DRAG_KEYS,
  FILL_KEYS,
  HOVER_KEYS,
  KEY_KEYS,
  markHandle,
  POINTER_KEYS,
  pick,
  SCREENSHOT_KEYS,
  selectOptionValues,
} from './options'
import type { Page } from './page'
import type { Response } from './response'
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

export class Frame extends EventEmitter {
  readonly _page: Page
  /** Rust's frame id; null for the main frame, which the wire leaves out. */
  readonly _id: string | null
  _name = ''
  _url = ''
  _parentFrame: Frame | null
  _detached = false

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
    return this._name
  }

  url(): string {
    return this._id === null ? this._page._url : this._url
  }

  parentFrame(): Frame | null {
    return this._parentFrame
  }

  childFrames(): Frame[] {
    return this._page.frames().filter((frame) => frame._parentFrame === this)
  }

  isDetached(): boolean {
    return this._detached
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
    // biome-ignore lint/complexity/noArguments: upstream counts the arguments passed, not the parameters.
    assertMaxArguments('frame.$eval', arguments.length, 3)
    const api = 'frame.$eval'
    return await this._evaluateOn(
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
    return this._waitForLoadState('frame.waitForLoadState', state, options)
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
    return this._page._waitForURL('frame.waitForURL', this, url, options)
  }

  async title(): Promise<string> {
    const api = 'frame.title'
    return await invoke(
      'page.title',
      this._call(api, 0),
      await this._frameArgs(api),
    )
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

  /** One op on the element `selector` names in this frame. */
  async _selectorOp<Op extends SelectorOp>(
    op: Op,
    api: string,
    selector: string,
    strict: boolean | undefined,
    wait: Wait,
    args: Omit<OpInputs[Op], 'target' | 'strict'>,
  ): Promise<OpOutputs[Op]> {
    const target = await this._locatorBrand(api, selector)
    const input = {
      target,
      strict: strict === true ? 'strict' : 'firstMatch',
      ...args,
    } as OpInputs[Op]
    return await invoke(op, this._call(api, wait.timeout), input, wait.signal)
  }

  async _click(
    api: string,
    selector: string,
    options: ClickOptions = {},
  ): Promise<void> {
    await this._selectorOp(
      'locator.click',
      api,
      selector,
      options.strict,
      this._wait(api, options),
      {
        options: pick(options, POINTER_KEYS),
      },
    )
  }

  async _dblclick(
    api: string,
    selector: string,
    options: ClickOptions = {},
  ): Promise<void> {
    await this._selectorOp(
      'locator.dblclick',
      api,
      selector,
      options.strict,
      this._wait(api, options),
      {
        options: pick(options, DBLCLICK_KEYS),
      },
    )
  }

  async _hover(
    api: string,
    selector: string,
    options: ClickOptions = {},
  ): Promise<void> {
    await this._selectorOp(
      'locator.hover',
      api,
      selector,
      options.strict,
      this._wait(api, options),
      {
        options: pick(options, HOVER_KEYS),
      },
    )
  }

  /** `check`, `uncheck` and `setChecked`, which upstream splits client-side the same way. */
  async _check(
    api: string,
    selector: string,
    checked: boolean,
    options: ClickOptions = {},
  ): Promise<void> {
    const op = checked ? 'locator.check' : 'locator.uncheck'
    await this._selectorOp(
      op,
      api,
      selector,
      options.strict,
      this._wait(api, options),
      {
        options: pick(options, CHECK_KEYS),
      },
    )
  }

  async _dragAndDrop(
    api: string,
    source: string,
    target: string,
    options: DragOptions & TimeoutOption & StrictOption = {},
  ): Promise<void> {
    // Read before the first await, as every action reads its signal (signal.ts).
    const wait = this._wait(api, options)
    const to = await this._locatorBrand(api, target)
    await this._selectorOp(
      'locator.dragTo',
      api,
      source,
      options.strict,
      wait,
      {
        to,
        options: pick(options, DRAG_KEYS),
      },
    )
  }

  async _fill(
    api: string,
    selector: string,
    value: string,
    options: FillOptions & TimeoutOption & StrictOption = {},
  ): Promise<void> {
    if (typeof value !== 'string')
      throw clientError(api, `value: expected string, got ${typeof value}`)
    await this._selectorOp(
      'locator.fill',
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
    await this._selectorOp(
      'locator.press',
      api,
      selector,
      options.strict,
      this._wait(api, options),
      {
        key,
        options: pick(options, KEY_KEYS),
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
    await this._selectorOp(
      'locator.pressSequentially',
      api,
      selector,
      options.strict,
      this._wait(api, options),
      { text, options: pick(options, KEY_KEYS) },
    )
  }

  async _focus(
    api: string,
    selector: string,
    options: TimeoutOption & StrictOption = {},
  ): Promise<void> {
    await this._selectorOp(
      'locator.focus',
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
    await this._selectorOp(
      'locator.blur',
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
    return await this._selectorOp(
      'locator.selectOption',
      api,
      selector,
      options.strict,
      this._wait(api, options),
      { ...selectOptionValues(api, values), ...pick(options, ['force']) },
    )
  }

  async _selectText(
    api: string,
    selector: string,
    options: TimeoutOption & StrictOption & { force?: boolean } = {},
  ): Promise<void> {
    await this._selectorOp(
      'locator.selectText',
      api,
      selector,
      options.strict,
      this._wait(api, options),
      pick(options, ['force']),
    )
  }

  async _setInputFiles(
    api: string,
    selector: string,
    files: unknown,
    options: TimeoutOption & StrictOption = {},
  ): Promise<void> {
    // Upstream's client converts the files before it sends the selector anywhere.
    const input = inputFiles(api, files)
    await withClientErrors(api, () =>
      this._selectorOp(
        'locator.setInputFiles',
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
    await this._selectorOp(
      'locator.dispatchEvent',
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
    await this._selectorOp(
      'locator.scrollIntoViewIfNeeded',
      api,
      selector,
      options.strict,
      this._wait(api, options),
      {},
    )
  }

  /** The reads that wait for one element, then return what the sealed world read. */
  async _read<
    Op extends
      | 'locator.textContent'
      | 'locator.innerText'
      | 'locator.innerHTML'
      | 'locator.inputValue'
      | 'locator.isChecked'
      | 'locator.isDisabled'
      | 'locator.isEditable'
      | 'locator.isEnabled',
  >(
    op: Op,
    api: string,
    selector: string,
    options: TimeoutOption & StrictOption = {},
  ): Promise<OpOutputs[Op]> {
    const args = {} as Omit<OpInputs[Op], 'target' | 'strict'>
    return await this._selectorOp(
      op,
      api,
      selector,
      options.strict,
      this._wait(api, options),
      args,
    )
  }

  async _getAttribute(
    api: string,
    selector: string,
    name: string,
    options: TimeoutOption & StrictOption = {},
  ): Promise<string | null> {
    return await this._selectorOp(
      'locator.getAttribute',
      api,
      selector,
      options.strict,
      this._wait(api, options),
      { name },
    )
  }

  /** `isVisible` and `isHidden` never wait, as upstream (their timeout is ignored). */
  async _isVisible(
    op: 'locator.isVisible' | 'locator.isHidden',
    api: string,
    selector: string,
    options: StrictOption = {},
  ): Promise<boolean> {
    return await this._selectorOp(
      op,
      api,
      selector,
      options.strict,
      { timeout: 0 },
      {},
    )
  }

  async _everyMatch<
    Op extends
      | 'locator.count'
      | 'locator.allTextContents'
      | 'locator.allInnerTexts',
  >(op: Op, api: string, selector: string): Promise<OpOutputs[Op]> {
    const target = await this._locatorBrand(api, selector)
    const input = { target } as OpInputs[Op]
    return await invoke(op, this._call(api, 0), input)
  }

  /**
   * Upstream's `resolveSelector`: the selector Playwright's generator writes for the element
   * `selector` matches now, from the main frame down through every `<iframe>` on the way.
   */
  async _normalize(api: string, selector: string): Promise<string> {
    const target = await this._locatorBrand(api, selector)
    const input = { target, selector }
    const resolved = await invoke(
      'locator.normalize',
      this._call(api, 0),
      input,
    )
    return resolved.selector
  }

  async _boundingBox(
    api: string,
    selector: string,
    options: TimeoutOption = {},
  ): Promise<OpOutputs['locator.boundingBox']> {
    return await this._selectorOp(
      'locator.boundingBox',
      api,
      selector,
      true,
      this._wait(api, options),
      {},
    )
  }

  async _waitForSelector(
    api: string,
    selector: string,
    options: WaitForSelectorOptions = {},
    omitReturnValue: boolean,
  ): Promise<ElementHandle | null> {
    const raw: unknown = options
    if (typeof raw === 'object' && raw !== null && 'visibility' in raw)
      throw clientError(
        api,
        'options.visibility is not supported, did you mean options.state?',
      )
    const state = options.state ?? 'visible'
    const handle = await this._selectorOp(
      'locator.waitFor',
      api,
      selector,
      options.strict,
      this._wait(api, options),
      { state, omitReturnValue },
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
    const handle = await this._selectorOp(
      'locator.elementHandle',
      api,
      selector,
      strict,
      noWait ? { timeout: 0 } : this._wait(api, options),
      noWait ? { noWait: true } : {},
    )
    return this._page._handle(handle)
  }

  async _elementHandles(
    api: string,
    selector: string,
  ): Promise<ElementHandle[]> {
    const target = await this._locatorBrand(api, selector)
    const handles = await invoke('locator.elementHandles', this._call(api, 0), {
      target,
    })
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
    selector: string,
    strict: boolean,
    script: PageScript,
    noWait: boolean,
    options: TimeoutOption & SignalOption = {},
  ): Promise<unknown> {
    const value = await this._selectorOp(
      'locator.evaluate',
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
    const target = await this._locatorBrand(api, selector)
    const value = await invoke('locator.evaluateAll', this._call(api, 0), {
      target,
      script,
    })
    return fromPageValue(api, value)
  }

  /** A snapshot rooted at `selector`, or at the page when `selector` is null. */
  async _snapshot(
    api: string,
    selector: string | null,
    options: SnapshotOptions = {},
  ): Promise<SnapshotResult> {
    const mode = options.mode ?? 'default'
    const shape = pick({ mode, depth: options.depth, boxes: options.boxes }, [
      'mode',
      'depth',
      'boxes',
    ])
    const wait = this._wait(api, options)
    if (selector !== null)
      return await this._selectorOp(
        'locator.ariaSnapshot',
        api,
        selector,
        true,
        wait,
        shape,
      )
    if (this._id !== null)
      throw unsupportedError(
        api,
        "snapshot a child frame through its iframe: page.locator('iframe').contentFrame().locator('body').ariaSnapshot()",
      )
    return await invoke(
      'page.ariaSnapshot',
      this._call(api, wait.timeout),
      { page: await this._page._brand(api), ...shape },
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
    if (options.path !== undefined)
      throw unsupportedError(
        api,
        'Neo returns the image instead of writing a file; omit path',
      )
    const refused = refusedScreenshotOption(options)
    if (refused !== undefined) throw unsupportedError(api, refused)
    const wire: ScreenshotOptions = pick(options, SCREENSHOT_KEYS)
    const { timeout, signal } = this._wait(api, options)
    const page = await this._page._brand(api)
    if (selector === null)
      return await invoke(
        'page.screenshot',
        this._call(api, timeout),
        { page, ...wire },
        signal,
      )
    const target = await this._locatorBrand(api, selector)
    return await invoke(
      'page.screenshot',
      this._call(api, timeout),
      { page, target, strict: 'strict', ...wire },
      signal,
    )
  }

  async _goto(
    api: string,
    url: string,
    options: NavigateOptions = {},
  ): Promise<Response | null> {
    if (typeof url !== 'string')
      throw clientError(api, `url: expected string, got ${typeof url}`)
    const waitUntil = loadState(api, 'waitUntil', options.waitUntil ?? 'load')
    const signal = signalOf(api, options)
    return await this._page._navigating(async () => {
      const args = await this._frameArgs(api)
      const { value: result, observed } = await invokeObserved(
        'page.goto',
        this._call(api, this._navigationTimeout(options)),
        {
          ...args,
          url,
          waitUntil,
          ...pick(options, ['referer']),
        },
        undefined,
        signal,
      )
      return await this._page._navigated(api, this, result, observed)
    })
  }

  async _waitForLoadState(
    api: string,
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
    return await invoke(
      'page.content',
      this._call(api, 0),
      await this._frameArgs(api),
    )
  }

  /** Page delegates here so frame routing, errors and returned node ownership stay identical. */
  async _addTag(
    api: string,
    op: 'page.addScriptTag' | 'page.addStyleTag',
    options: ScriptTagOptions,
  ): Promise<ElementHandle> {
    const keys =
      op === 'page.addScriptTag'
        ? (['url', 'path', 'content', 'type'] as const)
        : (['url', 'path', 'content'] as const)
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
    const waitUntil = loadState(api, 'waitUntil', options.waitUntil ?? 'load')
    const signal = signalOf(api, options)
    const { value: result, observed } = await invokeObserved(
      'page.setContent',
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
    const script = pageCall(api, 'frame', pageFunction, arg, options)
    const { value: result, observed } = await invokeObserved(
      'page.evaluate',
      this._call(api, 0),
      {
        ...(await this._frameArgs(api)),
        script,
      },
    )
    this._page._evaluated(result.frames, observed)
    return fromPageValue(api, result.value)
  }

  /** upstream's `Frame.waitForFunction`: its polling check, unprefixed, comes first. */
  async _waitForFunction(
    api: string,
    pageFunction: unknown,
    arg: unknown,
    options: WaitForFunctionOptions = {},
  ): Promise<WaitHandle> {
    if (typeof options.polling === 'string' && options.polling !== 'raf')
      throw plainError(api, `Unknown polling option: ${options.polling}`)
    const script = pageCall(api, 'frame', pageFunction, arg)
    const wait = this._wait(api, options)
    const { value, preview } = await invoke(
      'page.waitForFunction',
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
  ScreenshotOptions & {
    path?: string
    mask?: Locator[]
    maskColor?: string
  }

/** Options that need capture features beyond temporary DOM preparation. */
export function refusedScreenshotOption(
  options: ScreenshotOptionsInput,
): string | undefined {
  if (options.mask !== undefined && options.mask.length > 0)
    return 'Neo cannot mask elements in a screenshot; omit mask'
  return undefined
}

const HELD_VALUE =
  "Neo's waitForFunction handle holds the value, not the page's object; read it with jsonValue()"

/**
 * What `waitForFunction` resolves to: upstream's JSHandle for the truthy value
 * (`JSHandle.from`, PW:62147), as a snapshot. Neo keeps no page object past an op, so the
 * handle holds the value Rust read when the wait ended, masked, in Playwright's format, and
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
