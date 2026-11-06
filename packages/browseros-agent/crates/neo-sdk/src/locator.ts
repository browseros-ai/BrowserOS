/**
 * Playwright's `Locator` (`client/locator.ts` at 26137face). Every selector a locator holds is
 * built by Playwright's own code: the method bodies here are Playwright's, over the vendored,
 * unchanged `locatorUtils.ts` and `stringUtils.ts`. That is what makes Neo's selectors
 * byte-identical to Playwright's, inside QuickJS too (`neo-runtime/tests/selector_goldens.rs`
 * runs the 94 goldens in `tests/goldens/selector-goldens.json` through the realm).
 *
 * What differs from Playwright, all listed here:
 * - Imports point at the vendored copies; `isString` comes straight from `stringUtils.ts`,
 *   which Playwright's `rtti.ts` re-exports it from.
 * - The constructor drops Playwright's `inspect.custom` hook (Node's `util`, absent in QuickJS).
 * - `click` passes its `apiName`, `locator.click`, to `Frame._click`: Playwright reads it off V8
 *   stack frames, and QuickJS frames carry no class names (`connection.ts`).
 * - Absent: `frameLocator`, `contentFrame` (frames come later; calling one is a `TypeError`),
 *   `toString` and `description` (they need Playwright's ~2,500-line locator generators), and
 *   every action and read until Neo serves it. `_apiName` and `_wrapApiCall` have no use here.
 *
 * Frame identity is load-bearing: `has`, `hasNot`, `and`, `or` and `locator(Locator)` throw
 * unless both locators hold the same `Frame` object, which is why a page builds its main frame
 * once (`Page.mainFrame`).
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
import {
  escapeForTextSelector,
  isString,
} from '../vendor/playwright/isomorphic/stringUtils'
import type { ClickOptions, Frame } from './frame'
import type { Page } from './page'

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
