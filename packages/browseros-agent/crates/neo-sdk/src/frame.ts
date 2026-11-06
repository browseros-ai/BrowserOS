/**
 * Playwright's `Frame` (`client/frame.ts` at 26137face), cut to a page's main frame: its guid,
 * its page, the selector builders, whose bodies are Playwright's, and browser actions. A page builds
 * exactly one `Frame` per run, because `Locator` compares frames by identity.
 *
 * Private action helpers send each protocol message with its public caller's `apiName`.
 */

import type {
  FrameCheckParams,
  FrameClickParams,
  FrameFillParams,
  FrameInputValueParams,
  FramePressParams,
  FrameSelectOptionParams,
  FrameUncheckParams,
} from '@browseros/neo-runtime-spec/serve'
import type { ByRoleOptions } from '../vendor/playwright/isomorphic/locatorUtils'
import {
  getByAltTextSelector,
  getByLabelSelector,
  getByPlaceholderSelector,
  getByRoleSelector,
  getByTestIdSelector,
  getByTextSelector,
  getByTitleSelector,
} from '../vendor/playwright/isomorphic/locatorUtils'
import type { ApiName, Connection } from './connection'
import { Locator, type LocatorOptions, testIdAttributeName } from './locator'
import type { Page } from './page'
import {
  FRAME_CHECK_KEYS,
  FRAME_CLICK_KEYS,
  FRAME_FILL_KEYS,
  FRAME_INPUT_VALUE_KEYS,
  FRAME_PRESS_KEYS,
  FRAME_SELECT_OPTION_KEYS,
  FRAME_UNCHECK_KEYS,
  pickParams,
} from './protocol'
import {
  convertSelectOptionValues,
  type SelectOptionValues,
} from './select-option'

/** `click`'s options as a script passes them: the wire params plus `timeout`. */
export type ClickOptions = Omit<FrameClickParams, 'selector'> & {
  timeout?: number
}

export type FillOptions = Omit<FrameFillParams, 'selector' | 'value'> & {
  timeout?: number
}
export type PressOptions = Omit<FramePressParams, 'selector' | 'key'> & {
  timeout?: number
}
export type InputValueOptions = Omit<FrameInputValueParams, 'selector'> & {
  timeout?: number
}
export type CheckOptions = Omit<FrameCheckParams, 'selector'> & {
  timeout?: number
}
export type UncheckOptions = Omit<FrameUncheckParams, 'selector'> & {
  timeout?: number
}
export type SelectOptionOptions = Omit<
  FrameSelectOptionParams,
  'selector' | 'elements' | 'options'
> & { timeout?: number }

export class Frame {
  /** `frame@<targetId>`, the guid neo-runtime routes this frame's browser calls by. */
  readonly _guid: string
  readonly _page: Page
  readonly _connection: Connection

  constructor(guid: string, page: Page, connection: Connection) {
    this._guid = guid
    this._page = page
    this._connection = connection
  }

  page(): Page {
    return this._page
  }

  /** Not strict, as Playwright's `frame.click`. */
  async click(selector: string, options: ClickOptions = {}): Promise<void> {
    return await this._click('frame.click', selector, options)
  }

  /** Sends `Frame.click`. Runs synchronously into `Connection.send`, so the location it
   * captures is the script's call site. */
  async _click(
    apiName: ApiName,
    selector: string,
    options: ClickOptions,
  ): Promise<void> {
    await this._connection.send(
      apiName,
      this._guid,
      'click',
      pickParams(FRAME_CLICK_KEYS, {
        selector,
        ...options,
      }) as FrameClickParams,
      this._page._timeoutSettings.timeout(options),
    )
  }

  async fill(
    selector: string,
    value: string,
    options: FillOptions = {},
  ): Promise<void> {
    return await this._fill('frame.fill', selector, value, options)
  }

  /** Synchronously reaches `send` so it captures the script's call site. */
  async _fill(
    apiName: ApiName,
    selector: string,
    value: string,
    options: FillOptions,
  ): Promise<void> {
    await this._connection.send(
      apiName,
      this._guid,
      'fill',
      pickParams(FRAME_FILL_KEYS, {
        selector,
        value,
        ...options,
      }) as FrameFillParams,
      this._page._timeoutSettings.timeout(options),
    )
  }

  async press(
    selector: string,
    key: string,
    options: PressOptions = {},
  ): Promise<void> {
    return await this._press('frame.press', selector, key, options)
  }

  /** Synchronously reaches `send` so it captures the script's call site. */
  async _press(
    apiName: ApiName,
    selector: string,
    key: string,
    options: PressOptions,
  ): Promise<void> {
    await this._connection.send(
      apiName,
      this._guid,
      'press',
      pickParams(FRAME_PRESS_KEYS, {
        selector,
        key,
        ...options,
      }) as FramePressParams,
      this._page._timeoutSettings.timeout(options),
    )
  }

  async inputValue(
    selector: string,
    options: InputValueOptions = {},
  ): Promise<string> {
    return await this._inputValue('frame.inputValue', selector, options)
  }

  /** Synchronously reaches `send` so it captures the script's call site. */
  async _inputValue(
    apiName: ApiName,
    selector: string,
    options: InputValueOptions,
  ): Promise<string> {
    const result = await this._connection.send(
      apiName,
      this._guid,
      'inputValue',
      pickParams(FRAME_INPUT_VALUE_KEYS, {
        selector,
        ...options,
      }) as FrameInputValueParams,
      this._page._timeoutSettings.timeout(options),
    )
    return result!.value
  }

  async check(selector: string, options: CheckOptions = {}): Promise<void> {
    return await this._check('frame.check', selector, options)
  }

  /** Synchronously reaches `send` so it captures the script's call site. */
  async _check(
    apiName: ApiName,
    selector: string,
    options: CheckOptions,
  ): Promise<void> {
    await this._connection.send(
      apiName,
      this._guid,
      'check',
      pickParams(FRAME_CHECK_KEYS, {
        selector,
        ...options,
      }) as FrameCheckParams,
      this._page._timeoutSettings.timeout(options),
    )
  }

  async uncheck(selector: string, options: UncheckOptions = {}): Promise<void> {
    return await this._uncheck('frame.uncheck', selector, options)
  }

  /** Synchronously reaches `send` so it captures the script's call site. */
  async _uncheck(
    apiName: ApiName,
    selector: string,
    options: UncheckOptions,
  ): Promise<void> {
    await this._connection.send(
      apiName,
      this._guid,
      'uncheck',
      pickParams(FRAME_UNCHECK_KEYS, {
        selector,
        ...options,
      }) as FrameUncheckParams,
      this._page._timeoutSettings.timeout(options),
    )
  }

  async selectOption(
    selector: string,
    values: SelectOptionValues,
    options: SelectOptionOptions = {},
  ): Promise<string[]> {
    return await this._selectOption(
      'frame.selectOption',
      selector,
      values,
      options,
    )
  }

  /** Synchronously reaches `send` so it captures the script's call site. */
  async _selectOption(
    apiName: ApiName,
    selector: string,
    values: SelectOptionValues,
    options: SelectOptionOptions,
  ): Promise<string[]> {
    const result = await this._connection.send(
      apiName,
      this._guid,
      'selectOption',
      pickParams(FRAME_SELECT_OPTION_KEYS, {
        selector,
        ...convertSelectOptionValues(values),
        ...options,
      }) as FrameSelectOptionParams,
      this._page._timeoutSettings.timeout(options),
    )
    return result!.values
  }

  async setChecked(
    selector: string,
    checked: boolean,
    options: CheckOptions = {},
  ): Promise<void> {
    if (checked) await this._check('frame.setChecked', selector, options)
    else await this._uncheck('frame.setChecked', selector, options)
  }

  locator(selector: string, options?: LocatorOptions): Locator {
    return new Locator(this, selector, options)
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
}
