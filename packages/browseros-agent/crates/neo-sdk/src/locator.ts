/**
 * Playwright's `Locator` (`client/locator.ts` at 26137face). Every selector a locator holds is
 * built by Playwright's own code: the method bodies here are Playwright's, over the vendored,
 * unchanged `locatorUtils.ts` and `stringUtils.ts`. That is what makes Neo's selectors
 * byte-identical to Playwright's, inside QuickJS too (`neo-runtime/tests/selector_goldens.rs`
 * runs `tests/goldens/*selector-goldens.json` through the realm). `toString` and `description`
 * read a selector back with Playwright's vendored, unchanged `locatorGenerators.ts`.
 *
 * What differs from Playwright, all listed here:
 * - Imports point at the vendored copies; `isString` comes straight from `stringUtils.ts`,
 *   which Playwright's `rtti.ts` re-exports it from.
 * - The constructor drops Playwright's `inspect.custom` hook (Node's `util`, absent in QuickJS).
 * - Browser calls pass their public `apiName` to the corresponding `Frame` helper: Playwright reads
 *   it off V8 stack frames, and QuickJS frames carry no class names (`connection.ts`).
 * - Absent: actions and reads Neo does not serve. Explicit names preserve the outer `clear`,
 *   `setChecked` and `all` API names through their delegated calls (`_wrapApiCall` in
 *   Playwright).
 * - `waitFor` sends Playwright's own `Frame.waitForSelector` through `Frame._waitForSelector`,
 *   as Playwright's goes straight to the frame's channel.
 * - `ariaSnapshot` shares `Frame._ariaSnapshot` with page snapshots, sending this selector.
 * - `allTextContents` and `allInnerTexts` run as `$$eval` in the page's main world.
 *
 * Frame identity is load-bearing: `has`, `hasNot`, `and`, `or` and `locator(Locator)` throw
 * unless both locators hold the same `Frame` object, which is why a page interns every frame
 * by its Rust-minted guid.
 */

import {
  asLocatorDescription,
  locatorCustomDescription,
} from '../vendor/playwright/isomorphic/locatorGenerators'
import {
  type ByRoleOptions,
  getByAltTextSelector,
  getByLabelSelector,
  getByPlaceholderSelector,
  getByRoleSelector,
  getByTestIdSelector,
  getByTextSelector,
  getByTitleSelector,
} from '../vendor/playwright/isomorphic/locatorUtils'
import {
  escapeForTextSelector,
  isString,
} from '../vendor/playwright/isomorphic/stringUtils'
import type { Buffer } from './buffer'
import type { ElementHandle } from './element-handle'
import type {
  AriaSnapshotOptions,
  CheckOptions,
  ClickOptions,
  DblclickOptions,
  FillOptions,
  Frame,
  GetAttributeOptions,
  HoverOptions,
  InnerHTMLOptions,
  InnerTextOptions,
  InputValueOptions,
  IsCheckedOptions,
  IsDisabledOptions,
  IsEditableOptions,
  IsEnabledOptions,
  IsHiddenOptions,
  IsVisibleOptions,
  PressOptions,
  SelectOptionOptions,
  TapOptions,
  TextContentOptions,
  UncheckOptions,
  WaitForSelectorOptions,
} from './frame'
import { FrameLocator } from './frame-locator'
import type {
  EvaluateOptions,
  JSHandle,
  PageElement,
  PageFunctionOn,
} from './js-handle'
import type { Page } from './page'
import {
  locatorScreenshotParams,
  resultBuffer,
  type ScreenshotOptions,
} from './screenshot'
import type { SelectOptionValues } from './select-option'

export type LocatorOptions = {
  hasText?: string | RegExp
  hasNotText?: string | RegExp
  has?: Locator
  hasNot?: Locator
  visible?: boolean
}

export class Locator {
  _frame: Frame
  _selector: string
  /** Playwright's type tag, which its test matchers read; it also shows in `JSON.stringify`. */
  _apiName = 'Locator'

  constructor(frame: Frame, selector: string, options?: LocatorOptions) {
    this._frame = frame
    this._selector = selector

    if (options?.hasText)
      this._selector += ` >> internal:has-text=${escapeForTextSelector(options.hasText, false)}`

    if (options?.hasNotText)
      this._selector += ` >> internal:has-not-text=${escapeForTextSelector(options.hasNotText, false)}`

    if (options?.has) {
      const locator = options.has
      if (locator._frame !== frame)
        throw new Error(`Inner "has" locator must belong to the same frame.`)
      this._selector += ` >> internal:has=` + JSON.stringify(locator._selector)
    }

    if (options?.hasNot) {
      const locator = options.hasNot
      if (locator._frame !== frame)
        throw new Error(`Inner "hasNot" locator must belong to the same frame.`)
      this._selector +=
        ` >> internal:has-not=` + JSON.stringify(locator._selector)
    }

    if (options?.visible !== undefined)
      this._selector += ` >> visible=${options.visible ? 'true' : 'false'}`
  }

  _equals(locator: Locator) {
    return (
      this._frame === locator._frame && this._selector === locator._selector
    )
  }

  page(): Page {
    return this._frame.page()
  }

  async click(options: ClickOptions = {}): Promise<void> {
    return await this._frame._click('locator.click', this._selector, {
      strict: true,
      ...options,
    })
  }

  async dblclick(options: DblclickOptions = {}): Promise<void> {
    await this._frame._dblclick('locator.dblclick', this._selector, {
      strict: true,
      ...options,
    })
  }

  async hover(options: HoverOptions = {}): Promise<void> {
    return await this._frame._hover('locator.hover', this._selector, {
      strict: true,
      ...options,
    })
  }

  async tap(options: TapOptions = {}): Promise<void> {
    return await this._frame._tap('locator.tap', this._selector, {
      strict: true,
      ...options,
    })
  }

  async textContent(options?: TextContentOptions): Promise<string | null> {
    return await this._frame._textContent(
      'locator.textContent',
      this._selector,
      { strict: true, ...options },
    )
  }

  async innerText(options?: InnerTextOptions): Promise<string> {
    return await this._frame._innerText('locator.innerText', this._selector, {
      strict: true,
      ...options,
    })
  }

  async innerHTML(options?: InnerHTMLOptions): Promise<string> {
    return await this._frame._innerHTML('locator.innerHTML', this._selector, {
      strict: true,
      ...options,
    })
  }

  async getAttribute(
    name: string,
    options?: GetAttributeOptions,
  ): Promise<string | null> {
    return await this._frame._getAttribute(
      'locator.getAttribute',
      this._selector,
      name,
      { strict: true, ...options },
    )
  }

  /**
   * Playwright: `client/locator.ts` `Locator.ariaSnapshot`: `selector: this._selector,`
   * The matched subtree as YAML in the selected frame. Default mode waits for a match,
   * omits refs and leaves iframe contents unstitched. `mode: 'ai'` snapshots stitch iframe
   * content with refs rooted in the main frame, in a child frame too (its refs take the path
   * of iframes from the main frame); behind an iframe that gets no ref (hidden, no pointer
   * events) an AI snapshot that would hold refs throws. AI does not auto-wait: use `waitFor()`
   * first if needed. `depth` limits tree levels (0 means unlimited). AI refs work with
   * `page.locator('aria-ref=…')` until the next snapshot of their document or a navigation
   * replaces them. Every mode requires one match.
   */
  async ariaSnapshot(options: AriaSnapshotOptions = {}): Promise<string> {
    return await this._frame._ariaSnapshot(
      'locator.ariaSnapshot',
      options,
      this._selector,
    )
  }

  async count(): Promise<number> {
    return await this._frame._queryCount('locator.count', this._selector)
  }

  async allTextContents(): Promise<string[]> {
    return await this._frame._allTextContents(
      'locator.allTextContents',
      this._selector,
    )
  }

  async allInnerTexts(): Promise<string[]> {
    return await this._frame._allInnerTexts(
      'locator.allInnerTexts',
      this._selector,
    )
  }

  /** One locator per match when it is called: `nth(i)` locators, which resolve again on use. */
  async all(): Promise<Locator[]> {
    return new Array(
      await this._frame._queryCount('locator.all', this._selector),
    )
      .fill(0)
      .map((e, i) => this.nth(i))
  }

  async fill(value: string, options: FillOptions = {}): Promise<void> {
    return await this._frame._fill('locator.fill', this._selector, value, {
      strict: true,
      ...options,
    })
  }

  async press(key: string, options: PressOptions = {}): Promise<void> {
    return await this._frame._press('locator.press', this._selector, key, {
      strict: true,
      ...options,
    })
  }

  async inputValue(options: InputValueOptions = {}): Promise<string> {
    return await this._frame._inputValue('locator.inputValue', this._selector, {
      strict: true,
      ...options,
    })
  }

  async isVisible(options?: IsVisibleOptions): Promise<boolean> {
    return await this._frame._isVisible('locator.isVisible', this._selector, {
      strict: true,
      ...options,
    })
  }

  async isChecked(options?: IsCheckedOptions): Promise<boolean> {
    return await this._frame._isChecked('locator.isChecked', this._selector, {
      strict: true,
      ...options,
    })
  }

  /** Reads once, without waiting, as `isVisible`; `timeout` is ignored. */
  async isHidden(options?: IsHiddenOptions): Promise<boolean> {
    return await this._frame._isHidden('locator.isHidden', this._selector, {
      strict: true,
      ...options,
    })
  }

  async isEnabled(options?: IsEnabledOptions): Promise<boolean> {
    return await this._frame._isEnabled('locator.isEnabled', this._selector, {
      strict: true,
      ...options,
    })
  }

  async isDisabled(options?: IsDisabledOptions): Promise<boolean> {
    return await this._frame._isDisabled('locator.isDisabled', this._selector, {
      strict: true,
      ...options,
    })
  }

  async isEditable(options?: IsEditableOptions): Promise<boolean> {
    return await this._frame._isEditable('locator.isEditable', this._selector, {
      strict: true,
      ...options,
    })
  }

  async check(options: CheckOptions = {}): Promise<void> {
    return await this._frame._check('locator.check', this._selector, {
      strict: true,
      ...options,
    })
  }

  async uncheck(options: UncheckOptions = {}): Promise<void> {
    return await this._frame._uncheck('locator.uncheck', this._selector, {
      strict: true,
      ...options,
    })
  }

  async selectOption(
    values: SelectOptionValues,
    options: SelectOptionOptions = {},
  ): Promise<string[]> {
    return await this._frame._selectOption(
      'locator.selectOption',
      this._selector,
      values,
      { strict: true, ...options },
    )
  }

  /**
   * Playwright's `Locator.screenshot`: the one element this locator strictly matches, once
   * attached, scrolled into view, visible and stable, as image bytes. One `neo:screenshot` call
   * (`screenshot.ts`, `neo-commands.ts`).
   */
  async screenshot(options: ScreenshotOptions = {}): Promise<Buffer> {
    const result = await this._frame._connection.sendNeo(
      'locator.screenshot',
      this._frame._guid,
      'neo:screenshot',
      locatorScreenshotParams('locator.screenshot', this._selector, options),
      this._frame._page._timeoutSettings.timeout(options, 'locator.screenshot'),
    )
    return resultBuffer(result.binary)
  }

  /**
   * Waits until the locator's element reaches `state`, and resolves `undefined`:
   * - `attached`: an element matches; `detached`: none does;
   * - `visible` (the default): the matching element is visible; `hidden`: no element matches,
   *   or the one that does is hidden.
   *
   * Strict: multiple matches or an invalid selector throw an `Error` at once. Frame-entering
   * selectors resolve inside their selected frames. Runs under the action timeout and polls
   * the page. It never takes the tab's input, so a click or typing in the same run can satisfy it.
   */
  /**
   * Playwright's `locator.evaluate`: `pageFunction` on the one element this locator matches, in
   * the main world, after waiting under the action timeout for it to be attached
   * (`_withElement`). More than one match is a strict mode violation.
   */
  async evaluate<R, Arg>(
    pageFunction: PageFunctionOn<PageElement, Arg, R>,
    arg?: Arg,
    options?: EvaluateOptions & { timeout?: number; signal?: unknown },
  ): Promise<R> {
    return await this._frame._withElement(
      'locator.evaluate',
      this._selector,
      (handle) =>
        handle._evaluate<R>('locator.evaluate', pageFunction, arg, options),
      { timeout: options?.timeout, signal: options?.signal },
    )
  }

  /** Playwright's `locator.evaluateHandle`: as `evaluate`, resolving a handle to the result. */
  async evaluateHandle<R, Arg>(
    pageFunction: PageFunctionOn<PageElement, Arg, R>,
    arg?: Arg,
    options?: EvaluateOptions & { timeout?: number; signal?: unknown },
  ): Promise<JSHandle<R>> {
    return await this._frame._withElement(
      'locator.evaluateHandle',
      this._selector,
      (handle) =>
        handle._evaluateHandle<R>(
          'locator.evaluateHandle',
          pageFunction,
          arg,
          options,
        ),
      { timeout: options?.timeout, signal: options?.signal },
    )
  }

  /** Playwright's `locator.evaluateAll`: `pageFunction` on the array of every match, without
   * waiting (`frame.$$eval`). */
  async evaluateAll<R, Arg>(
    pageFunction: PageFunctionOn<PageElement[], Arg, R>,
    arg?: Arg,
  ): Promise<R> {
    return await this._frame._evalOnSelectorAll(
      'locator.evaluateAll',
      this._selector,
      pageFunction,
      arg,
    )
  }

  /** Playwright's `locator.elementHandle`: the one element, once attached, as a handle. */
  async elementHandle(options?: {
    timeout?: number
    signal?: unknown
  }): Promise<ElementHandle> {
    const handle = await this._frame._waitForSelectorElement(
      'locator.elementHandle',
      this._selector,
      { strict: true, state: 'attached', ...options },
    )
    if (!handle)
      throw new Error(`Could not resolve ${this._selector} to DOM Element`)
    return handle
  }

  /** Playwright's `locator.elementHandles`: every match, as handles, without waiting. */
  async elementHandles(): Promise<ElementHandle[]> {
    return await this._frame._querySelectorAll(
      'locator.elementHandles',
      this._selector,
    )
  }

  async waitFor(options?: WaitForSelectorOptions): Promise<void> {
    await this._frame._waitForSelector('locator.waitFor', this._selector, {
      strict: true,
      omitReturnValue: true,
      ...options,
    })
  }

  async clear(options: FillOptions = {}): Promise<void> {
    return await this._frame._fill('locator.clear', this._selector, '', {
      strict: true,
      ...options,
    })
  }

  async setChecked(
    checked: boolean,
    options: CheckOptions = {},
  ): Promise<void> {
    if (checked)
      await this._frame._check('locator.setChecked', this._selector, {
        strict: true,
        ...options,
      })
    else
      await this._frame._uncheck('locator.setChecked', this._selector, {
        strict: true,
        ...options,
      })
  }

  locator(
    selectorOrLocator: string | Locator,
    options?: Omit<LocatorOptions, 'visible'>,
  ): Locator {
    if (isString(selectorOrLocator))
      return new Locator(
        this._frame,
        this._selector + ' >> ' + selectorOrLocator,
        options,
      )
    if (selectorOrLocator._frame !== this._frame)
      throw new Error(`Locators must belong to the same frame.`)
    return new Locator(
      this._frame,
      this._selector +
        ' >> internal:chain=' +
        JSON.stringify(selectorOrLocator._selector),
      options,
    )
  }

  within(locator: Locator): Locator {
    return locator.locator(this)
  }

  frameLocator(selector: string): FrameLocator {
    return new FrameLocator(this._frame, this._selector + ' >> ' + selector)
  }

  contentFrame(): FrameLocator {
    return new FrameLocator(this._frame, this._selector)
  }

  getByTestId(testId: string | RegExp): Locator {
    return this.locator(getByTestIdSelector(testIdAttributeName(), testId))
  }

  getByAltText(text: string | RegExp, options?: { exact?: boolean }): Locator {
    return this.locator(getByAltTextSelector(text, options))
  }

  getByLabel(text: string | RegExp, options?: { exact?: boolean }): Locator {
    return this.locator(getByLabelSelector(text, options))
  }

  getByPlaceholder(
    text: string | RegExp,
    options?: { exact?: boolean },
  ): Locator {
    return this.locator(getByPlaceholderSelector(text, options))
  }

  getByText(text: string | RegExp, options?: { exact?: boolean }): Locator {
    return this.locator(getByTextSelector(text, options))
  }

  getByTitle(text: string | RegExp, options?: { exact?: boolean }): Locator {
    return this.locator(getByTitleSelector(text, options))
  }

  getByRole(role: string, options: ByRoleOptions = {}): Locator {
    return this.locator(getByRoleSelector(role, options))
  }

  filter(options?: LocatorOptions): Locator {
    return new Locator(this._frame, this._selector, options)
  }

  visible(): Locator {
    return new Locator(this._frame, this._selector, { visible: true })
  }

  describe(description: string) {
    return new Locator(
      this._frame,
      this._selector + ' >> internal:describe=' + JSON.stringify(description),
    )
  }

  /** The last `describe()` text, or `null`: no page call. */
  description(): string | null {
    return locatorCustomDescription(this._selector) || null
  }

  first(): Locator {
    return new Locator(this._frame, this._selector + ' >> nth=0')
  }

  last(): Locator {
    return new Locator(this._frame, this._selector + ` >> nth=-1`)
  }

  nth(index: number): Locator {
    return new Locator(this._frame, this._selector + ` >> nth=${index}`)
  }

  and(locator: Locator): Locator {
    if (locator._frame !== this._frame)
      throw new Error(`Locators must belong to the same frame.`)
    return new Locator(
      this._frame,
      this._selector + ` >> internal:and=` + JSON.stringify(locator._selector),
    )
  }

  or(locator: Locator): Locator {
    if (locator._frame !== this._frame)
      throw new Error(`Locators must belong to the same frame.`)
    return new Locator(
      this._frame,
      this._selector + ` >> internal:or=` + JSON.stringify(locator._selector),
    )
  }

  /** The locator as Playwright's JavaScript builder text (`getByRole('button')`), or its
   * description when it has one; a selector the generator cannot read comes back as is. */
  toString() {
    return asLocatorDescription('javascript', this._selector)
  }
}

// Playwright's module-level test id attribute. `createRun` sets it from neo-runtime's
// `world::TEST_ID_ATTRIBUTE`, the attribute the page's injected script matches, so the selectors
// built here and the page agree on one name.
let _testIdAttributeName: string = 'data-testid'

export function testIdAttributeName(): string {
  return _testIdAttributeName
}

export function setTestIdAttribute(attributeName: string) {
  _testIdAttributeName = attributeName
}
