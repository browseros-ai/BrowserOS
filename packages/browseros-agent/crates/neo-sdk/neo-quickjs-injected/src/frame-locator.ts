/**
 * Playwright's `FrameLocator` (`client/locator.ts` at 26137face). Its method bodies build
 * selectors byte for byte as Playwright's, including nested hops and owner locators.
 *
 * What differs: imports use Neo's Frame and Locator and the unchanged vendored helpers;
 * `kAnyFrameSelector` is copied here to avoid importing the full selector parser. Rust rejects
 * that selector with a Neo error. A frame locator itself does no browser I/O.
 */
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
import { isString } from '../vendor/playwright/isomorphic/stringUtils'
import type { Frame } from './frame'
import { Locator, type LocatorOptions, testIdAttributeName } from './locator'

/** Playwright's `isomorphic/selectorParser.ts` constant. */
export const kAnyFrameSelector = 'internal:control=any-frame'

export class FrameLocator {
  private _frame: Frame
  private _frameSelector: string

  constructor(frame: Frame, selector: string) {
    this._frame = frame
    this._frameSelector = selector
  }

  private _childSelector(selector: string): string {
    if (this._frameSelector === kAnyFrameSelector)
      return this._frameSelector + ' >> ' + selector
    return (
      this._frameSelector + ' >> internal:control=enter-frame >> ' + selector
    )
  }

  locator(
    selectorOrLocator: string | Locator,
    options?: LocatorOptions,
  ): Locator {
    if (isString(selectorOrLocator))
      return new Locator(
        this._frame,
        this._childSelector(selectorOrLocator),
        options,
      )
    if (selectorOrLocator._frame !== this._frame)
      throw new Error(`Locators must belong to the same frame.`)
    return new Locator(
      this._frame,
      this._childSelector(selectorOrLocator._selector),
      options,
    )
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

  owner() {
    return new Locator(this._frame, this._frameSelector)
  }

  frameLocator(selector: string): FrameLocator {
    return new FrameLocator(this._frame, this._childSelector(selector))
  }

  private _nthSelector(nth: string): string {
    if (this._frameSelector === kAnyFrameSelector)
      throw new Error(
        `Selecting the nth frame is not allowed on frameLocator().`,
      )
    return this._frameSelector + ` >> nth=${nth}`
  }

  first(): FrameLocator {
    return new FrameLocator(this._frame, this._nthSelector('0'))
  }

  last(): FrameLocator {
    return new FrameLocator(this._frame, this._nthSelector('-1'))
  }

  nth(index: number): FrameLocator {
    return new FrameLocator(this._frame, this._nthSelector(String(index)))
  }
}
