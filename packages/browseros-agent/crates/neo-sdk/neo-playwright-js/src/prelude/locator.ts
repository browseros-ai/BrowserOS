/**
 * Playwright's Locator and FrameLocator, following playwright-core 1.64.1's client/locator.ts.
 *
 * A Locator is a value: its frame and its selector string. Builders compose strings exactly as
 * upstream does (tests/selector-goldens.test.ts checks every recorded case), and every action
 * or read delegates to the frame with `strict: true` and its own `locator.*` name. Rust
 * re-resolves the selector on every attempt, as Playwright's server does.
 */

import type { Rect } from '../probe/protocol'
import type { ElementHandle } from './element-handle'
import { internalError, plainError } from './errors'
import { type EvaluateOptions, pageCall } from './evaluation'
import type {
  ClickOptions,
  ExactOption,
  Frame,
  ScreenshotOptionsInput,
  SnapshotOptions,
  TextMatch,
} from './frame'
import { inspectCustom, markHandle } from './options'
import type { Page } from './page'
import { type LocatorOptions, withLocatorOptions } from './selectors'
import type { SignalOption } from './signal'
import type { TimeoutOption } from './timeouts'
import {
  type AriaSnapshotJSON,
  asLocatorDescription,
  type ByRoleOptions,
  getByAltTextSelector,
  getByLabelSelector,
  getByPlaceholderSelector,
  getByRoleSelector,
  getByTestIdSelector,
  getByTextSelector,
  getByTitleSelector,
  kAnyFrameSelector,
  locatorCustomDescription,
} from './vendored'
import type { DragOptions, FillOptions, KeyOptions, WaitForState } from './wire'

/** The attribute `getByTestId` matches; upstream's default, which Neo never changes. */
export function testIdAttributeName(): string {
  return 'data-testid'
}

export class Locator {
  readonly _frame: Frame
  readonly _selector: string

  /**
   * `selector` is the whole selector: a method that takes the script's options composes them
   * first (`withLocatorOptions`), so its refusal names that method.
   */
  constructor(frame: Frame, selector: string) {
    this._frame = frame
    this._selector = selector
    markHandle(this)
  }

  page(): Page {
    return this._frame.page()
  }

  toString(): string {
    return asLocatorDescription('javascript', this._selector)
  }

  [inspectCustom](): string {
    return this.toString()
  }

  description(): string | null {
    return locatorCustomDescription(this._selector) ?? null
  }

  // ---- builders: pure values ------------------------------------------------------------

  locator(
    selectorOrLocator: string | Locator,
    options?: Omit<LocatorOptions, 'visible'>,
  ): Locator {
    const api = 'locator.locator'
    if (typeof selectorOrLocator === 'string')
      return new Locator(
        this._frame,
        withLocatorOptions(
          api,
          this._frame,
          `${this._selector} >> ${selectorOrLocator}`,
          options,
        ),
      )
    if (selectorOrLocator._frame !== this._frame)
      throw plainError(api, 'Locators must belong to the same frame.')
    return new Locator(
      this._frame,
      withLocatorOptions(
        api,
        this._frame,
        `${this._selector} >> internal:chain=${JSON.stringify(selectorOrLocator._selector)}`,
        options,
      ),
    )
  }

  within(locator: Locator): Locator {
    return locator.locator(this)
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

  frameLocator(selector: string): FrameLocator {
    return new FrameLocator(this._frame, `${this._selector} >> ${selector}`)
  }

  contentFrame(): FrameLocator {
    return new FrameLocator(this._frame, this._selector)
  }

  filter(options?: LocatorOptions): Locator {
    return new Locator(
      this._frame,
      withLocatorOptions(
        'locator.filter',
        this._frame,
        this._selector,
        options,
      ),
    )
  }

  visible(): Locator {
    return new Locator(
      this._frame,
      withLocatorOptions('locator.visible', this._frame, this._selector, {
        visible: true,
      }),
    )
  }

  describe(description: string): Locator {
    return new Locator(
      this._frame,
      `${this._selector} >> internal:describe=${JSON.stringify(description)}`,
    )
  }

  first(): Locator {
    return new Locator(this._frame, `${this._selector} >> nth=0`)
  }

  last(): Locator {
    return new Locator(this._frame, `${this._selector} >> nth=-1`)
  }

  nth(index: number): Locator {
    return new Locator(this._frame, `${this._selector} >> nth=${index}`)
  }

  and(locator: Locator): Locator {
    if (locator._frame !== this._frame)
      throw plainError('locator.and', 'Locators must belong to the same frame.')
    return new Locator(
      this._frame,
      `${this._selector} >> internal:and=${JSON.stringify(locator._selector)}`,
    )
  }

  or(locator: Locator): Locator {
    if (locator._frame !== this._frame)
      throw plainError('locator.or', 'Locators must belong to the same frame.')
    return new Locator(
      this._frame,
      `${this._selector} >> internal:or=${JSON.stringify(locator._selector)}`,
    )
  }

  // ---- actions --------------------------------------------------------------------------

  click(options: ClickOptions = {}): Promise<void> {
    return this._frame._click('locator.click', this._selector, {
      ...options,
      strict: true,
    })
  }

  dblclick(options: ClickOptions = {}): Promise<void> {
    return this._frame._dblclick('locator.dblclick', this._selector, {
      ...options,
      strict: true,
    })
  }

  hover(options: ClickOptions = {}): Promise<void> {
    return this._frame._hover('locator.hover', this._selector, {
      ...options,
      strict: true,
    })
  }

  check(options: ClickOptions = {}): Promise<void> {
    return this._frame._check('locator.check', this._selector, true, {
      ...options,
      strict: true,
    })
  }

  uncheck(options: ClickOptions = {}): Promise<void> {
    return this._frame._check('locator.uncheck', this._selector, false, {
      ...options,
      strict: true,
    })
  }

  setChecked(checked: boolean, options: ClickOptions = {}): Promise<void> {
    return this._frame._check('locator.setChecked', this._selector, checked, {
      ...options,
      strict: true,
    })
  }

  /** `target` resolves in this locator's frame, as upstream passes its selector there. */
  dragTo(
    target: Locator,
    options: DragOptions & TimeoutOption = {},
  ): Promise<void> {
    return this._frame._dragAndDrop(
      'locator.dragTo',
      this._selector,
      target._selector,
      {
        ...options,
        strict: true,
      },
    )
  }

  fill(
    value: string,
    options: FillOptions & TimeoutOption = {},
  ): Promise<void> {
    return this._frame._fill('locator.fill', this._selector, value, {
      ...options,
      strict: true,
    })
  }

  clear(options: FillOptions & TimeoutOption = {}): Promise<void> {
    return this._frame._fill('locator.clear', this._selector, '', {
      ...options,
      strict: true,
    })
  }

  press(key: string, options: KeyOptions & TimeoutOption = {}): Promise<void> {
    return this._frame._press('locator.press', this._selector, key, {
      ...options,
      strict: true,
    })
  }

  pressSequentially(
    text: string,
    options: KeyOptions & TimeoutOption = {},
  ): Promise<void> {
    return this._frame._type(
      'locator.pressSequentially',
      this._selector,
      text,
      {
        ...options,
        strict: true,
      },
    )
  }

  type(text: string, options: KeyOptions & TimeoutOption = {}): Promise<void> {
    return this._frame._type('locator.type', this._selector, text, {
      ...options,
      strict: true,
    })
  }

  selectOption(
    values: unknown,
    options: TimeoutOption & { force?: boolean } = {},
  ): Promise<string[]> {
    return this._frame._selectOption(
      'locator.selectOption',
      this._selector,
      values,
      {
        ...options,
        strict: true,
      },
    )
  }

  selectText(options: TimeoutOption & { force?: boolean } = {}): Promise<void> {
    return this._frame._selectText('locator.selectText', this._selector, {
      ...options,
      strict: true,
    })
  }

  focus(options: TimeoutOption = {}): Promise<void> {
    return this._frame._focus('locator.focus', this._selector, {
      ...options,
      strict: true,
    })
  }

  blur(options: TimeoutOption = {}): Promise<void> {
    return this._frame._blur('locator.blur', this._selector, {
      ...options,
      strict: true,
    })
  }

  setInputFiles(files: unknown, options: TimeoutOption = {}): Promise<void> {
    return this._frame._setInputFiles(
      'locator.setInputFiles',
      this._selector,
      files,
      {
        ...options,
        strict: true,
      },
    )
  }

  dispatchEvent(
    type: string,
    eventInit: unknown = {},
    options: TimeoutOption = {},
  ): Promise<void> {
    return this._frame._dispatchEvent(
      'locator.dispatchEvent',
      this._selector,
      type,
      eventInit,
      {
        ...options,
        strict: true,
      },
    )
  }

  scrollIntoViewIfNeeded(options: TimeoutOption = {}): Promise<void> {
    return this._frame._scrollIntoViewIfNeeded(
      'locator.scrollIntoViewIfNeeded',
      this._selector,
      {
        ...options,
        strict: true,
      },
    )
  }

  // ---- reads ----------------------------------------------------------------------------

  textContent(options: TimeoutOption = {}): Promise<string | null> {
    return this._frame._read(
      'locator.textContent',
      'locator.textContent',
      this._selector,
      {
        ...options,
        strict: true,
      },
    )
  }

  innerText(options: TimeoutOption = {}): Promise<string> {
    return this._frame._read(
      'locator.innerText',
      'locator.innerText',
      this._selector,
      {
        ...options,
        strict: true,
      },
    )
  }

  innerHTML(options: TimeoutOption = {}): Promise<string> {
    return this._frame._read(
      'locator.innerHTML',
      'locator.innerHTML',
      this._selector,
      {
        ...options,
        strict: true,
      },
    )
  }

  inputValue(options: TimeoutOption = {}): Promise<string> {
    return this._frame._read(
      'locator.inputValue',
      'locator.inputValue',
      this._selector,
      {
        ...options,
        strict: true,
      },
    )
  }

  getAttribute(
    name: string,
    options: TimeoutOption = {},
  ): Promise<string | null> {
    return this._frame._getAttribute(
      'locator.getAttribute',
      this._selector,
      name,
      {
        ...options,
        strict: true,
      },
    )
  }

  isChecked(options: TimeoutOption = {}): Promise<boolean> {
    return this._frame._read(
      'locator.isChecked',
      'locator.isChecked',
      this._selector,
      {
        ...options,
        strict: true,
      },
    )
  }

  isDisabled(options: TimeoutOption = {}): Promise<boolean> {
    return this._frame._read(
      'locator.isDisabled',
      'locator.isDisabled',
      this._selector,
      {
        ...options,
        strict: true,
      },
    )
  }

  isEditable(options: TimeoutOption = {}): Promise<boolean> {
    return this._frame._read(
      'locator.isEditable',
      'locator.isEditable',
      this._selector,
      {
        ...options,
        strict: true,
      },
    )
  }

  isEnabled(options: TimeoutOption = {}): Promise<boolean> {
    return this._frame._read(
      'locator.isEnabled',
      'locator.isEnabled',
      this._selector,
      {
        ...options,
        strict: true,
      },
    )
  }

  isHidden(_options?: TimeoutOption): Promise<boolean> {
    return this._frame._isVisible(
      'locator.isHidden',
      'locator.isHidden',
      this._selector,
      {
        strict: true,
      },
    )
  }

  isVisible(_options?: TimeoutOption): Promise<boolean> {
    return this._frame._isVisible(
      'locator.isVisible',
      'locator.isVisible',
      this._selector,
      {
        strict: true,
      },
    )
  }

  count(): Promise<number> {
    return this._frame._everyMatch(
      'locator.count',
      'locator.count',
      this._selector,
    )
  }

  /** Upstream's: the same element, as the selector Playwright's generator writes for it. */
  async normalize(): Promise<Locator> {
    const resolved = await this._frame._normalize(
      'locator.normalize',
      this._selector,
    )
    return new Locator(this._frame, resolved)
  }

  async all(): Promise<Locator[]> {
    const count = await this._frame._everyMatch(
      'locator.count',
      'locator.all',
      this._selector,
    )
    return Array.from({ length: count }, (_, index) => this.nth(index))
  }

  allTextContents(): Promise<string[]> {
    return this._frame._everyMatch(
      'locator.allTextContents',
      'locator.allTextContents',
      this._selector,
    )
  }

  allInnerTexts(): Promise<string[]> {
    return this._frame._everyMatch(
      'locator.allInnerTexts',
      'locator.allInnerTexts',
      this._selector,
    )
  }

  boundingBox(options: TimeoutOption = {}): Promise<Rect | null> {
    return this._frame._boundingBox(
      'locator.boundingBox',
      this._selector,
      options,
    )
  }

  async waitFor(
    options: TimeoutOption & { state?: WaitForState } = {},
  ): Promise<void> {
    await this._frame._waitForSelector(
      'locator.waitFor',
      this._selector,
      { ...options, strict: true },
      true,
    )
  }

  /** Waits for the one element to be attached, then pins it, as upstream's waitForSelector. */
  async elementHandle(options: TimeoutOption = {}): Promise<ElementHandle> {
    const handle = await this._frame._elementHandle(
      'locator.elementHandle',
      this._selector,
      true,
      false,
      options,
    )
    if (!handle)
      throw internalError(
        'locator.elementHandle',
        `Could not resolve ${this} to DOM Element`,
      )
    return handle
  }

  elementHandles(): Promise<ElementHandle[]> {
    return this._frame._elementHandles('locator.elementHandles', this._selector)
  }

  /**
   * upstream's `_withElement`: the wait comes first, and `JSHandle.evaluate`'s checks run on the
   * handle it found, inside the same call, so they fail after the wait with the prefix. Its
   * `signal` aborts the one op that both waits and evaluates.
   */
  async evaluate(
    pageFunction: unknown,
    arg?: unknown,
    options?: TimeoutOption & EvaluateOptions & SignalOption,
  ): Promise<unknown> {
    const api = 'locator.evaluate'
    const script = pageCall(api, 'locator', pageFunction, arg, options)
    return await this._frame._evaluateOn(
      api,
      this._selector,
      true,
      script,
      false,
      typeof options === 'object' && options !== null ? options : {},
    )
  }

  /**
   * upstream's `Locator.waitForFunction`: `pageFunction(element, arg)` until it is truthy, the
   * locator matched afresh on every try and strict, within the call's timeout. The argument is
   * serialized now, as upstream's client does; Rust tries the predicate in the page. Resolves
   * to nothing.
   */
  async waitForFunction(
    pageFunction: unknown,
    arg?: unknown,
    options?: TimeoutOption & SignalOption,
  ): Promise<void> {
    const api = 'locator.waitForFunction'
    const script = pageCall(api, 'query', pageFunction, arg)
    await this._frame._selectorOp(
      'locator.waitForFunction',
      api,
      this._selector,
      true,
      this._frame._wait(api, options ?? {}),
      { script },
    )
  }

  /** upstream's `frame.$$eval` on this locator's selector, which counts its own arguments. */
  async evaluateAll(pageFunction: unknown, arg?: unknown): Promise<unknown> {
    const api = 'locator.evaluateAll'
    return await this._frame._evaluateAll(
      api,
      this._selector,
      pageCall(api, 'query', pageFunction, arg),
    )
  }

  ariaSnapshot(options?: SnapshotOptions): Promise<string> {
    return this._frame._ariaSnapshot(
      'locator.ariaSnapshot',
      this._selector,
      options,
    )
  }

  ariaSnapshotJSON(options?: SnapshotOptions): Promise<AriaSnapshotJSON> {
    return this._frame._ariaSnapshotJSON(
      'locator.ariaSnapshotJSON',
      this._selector,
      options,
    )
  }

  screenshot(options?: ScreenshotOptionsInput): Promise<unknown> {
    return this._frame._screenshot(
      'locator.screenshot',
      this._selector,
      options,
    )
  }
}

export class FrameLocator {
  readonly _frame: Frame
  readonly _frameSelector: string

  constructor(frame: Frame, selector: string) {
    this._frame = frame
    this._frameSelector = selector
    markHandle(this)
  }

  #childSelector(selector: string): string {
    if (this._frameSelector === kAnyFrameSelector)
      return `${this._frameSelector} >> ${selector}`
    return `${this._frameSelector} >> internal:control=enter-frame >> ${selector}`
  }

  locator(
    selectorOrLocator: string | Locator,
    options?: LocatorOptions,
  ): Locator {
    const api = 'frameLocator.locator'
    if (typeof selectorOrLocator === 'string')
      return new Locator(
        this._frame,
        withLocatorOptions(
          api,
          this._frame,
          this.#childSelector(selectorOrLocator),
          options,
        ),
      )
    if (selectorOrLocator._frame !== this._frame)
      throw plainError(api, 'Locators must belong to the same frame.')
    return new Locator(
      this._frame,
      withLocatorOptions(
        api,
        this._frame,
        this.#childSelector(selectorOrLocator._selector),
        options,
      ),
    )
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

  owner(): Locator {
    return new Locator(this._frame, this._frameSelector)
  }

  frameLocator(selector: string): FrameLocator {
    return new FrameLocator(this._frame, this.#childSelector(selector))
  }

  #nthSelector(api: string, nth: string): string {
    if (this._frameSelector === kAnyFrameSelector)
      throw plainError(
        api,
        'Selecting the nth frame is not allowed on frameLocator().',
      )
    return `${this._frameSelector} >> nth=${nth}`
  }

  first(): FrameLocator {
    return new FrameLocator(
      this._frame,
      this.#nthSelector('frameLocator.first', '0'),
    )
  }

  last(): FrameLocator {
    return new FrameLocator(
      this._frame,
      this.#nthSelector('frameLocator.last', '-1'),
    )
  }

  nth(index: number): FrameLocator {
    return new FrameLocator(
      this._frame,
      this.#nthSelector('frameLocator.nth', String(index)),
    )
  }
}
