/**
 * Playwright's `Frame` (`client/frame.ts` at 26137face), cut to a page's main frame: its guid,
 * its page, the selector builders, whose bodies are Playwright's, `goto`, `title`,
 * `textContent`, `innerText`, `getAttribute` and browser actions. A page builds exactly one
 * `Frame` per run, because `Locator` compares frames by identity.
 *
 * Each `_<method>` is where every public form of a browser call (`locator.textContent`,
 * `page.textContent`, `frame.textContent`) becomes a protocol message, with its caller's
 * `apiName`. Each runs synchronously into `Connection.send`, so the location it captures is the
 * script's call site.
 *
 * What differs from Playwright: `goto` resolves `null` for every navigation, because Neo creates
 * no `Response` objects yet (Playwright returns `null` only for navigations without one, such as
 * `about:blank` or a same-document change).
 */

import type {
  FrameAriaSnapshotParams,
  FrameAriaSnapshotResult,
  FrameCheckParams,
  FrameClickParams,
  FrameFillParams,
  FrameGetAttributeParams,
  FrameGetAttributeResult,
  FrameGotoParams,
  FrameInnerTextParams,
  FrameInnerTextResult,
  FrameInputValueParams,
  FramePressParams,
  FrameSelectOptionParams,
  FrameTextContentParams,
  FrameTextContentResult,
  FrameTitleResult,
  FrameUncheckParams,
  LifecycleEvent,
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
  FRAME_ARIA_SNAPSHOT_KEYS,
  FRAME_CHECK_KEYS,
  FRAME_CLICK_KEYS,
  FRAME_FILL_KEYS,
  FRAME_GET_ATTRIBUTE_KEYS,
  FRAME_GOTO_KEYS,
  FRAME_INNER_TEXT_KEYS,
  FRAME_INPUT_VALUE_KEYS,
  FRAME_PRESS_KEYS,
  FRAME_SELECT_OPTION_KEYS,
  FRAME_TEXT_CONTENT_KEYS,
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

/** `goto`'s options as a script passes them: the wire params plus `timeout`. */
export type GotoOptions = Omit<FrameGotoParams, 'url'> & { timeout?: number }

/** `textContent`'s options as a script passes them. */
export type TextContentOptions = Omit<FrameTextContentParams, 'selector'> & {
  timeout?: number
}

/** `innerText`'s options as a script passes them. */
export type InnerTextOptions = Omit<FrameInnerTextParams, 'selector'> & {
  timeout?: number
}

/** `getAttribute`'s options as a script passes them. */
export type GetAttributeOptions = Omit<
  FrameGetAttributeParams,
  'selector' | 'name'
> & {
  timeout?: number
}

/** `page.ariaSnapshot`'s options, as Playwright's `Page.ariaSnapshot` takes them. */
export type AriaSnapshotOptions = {
  mode?: 'ai' | 'default'
  depth?: number
  boxes?: boolean
  timeout?: number
}

/** Playwright's `kLifecycleEvents` (`client/types.ts`). */
const LIFECYCLE_EVENTS: ReadonlySet<string> = new Set([
  'load',
  'domcontentloaded',
  'networkidle',
  'commit',
])

/**
 * Playwright's `verifyLoadState` (`client/frame.ts`): `networkidle0` is an alias, and any other
 * unknown value throws a plain `Error` before a message is sent, so it has no `page.goto: `
 * prefix.
 */
export function verifyLoadState(
  name: string,
  waitUntil: LifecycleEvent,
): LifecycleEvent {
  if ((waitUntil as unknown) === 'networkidle0') waitUntil = 'networkidle'
  if (!LIFECYCLE_EVENTS.has(waitUntil))
    throw new Error(
      `${name}: expected one of (load|domcontentloaded|networkidle|commit)`,
    )
  return waitUntil
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

  /** Resolves `null`: Neo creates no `Response` objects yet. */
  async goto(url: string, options: GotoOptions = {}): Promise<null> {
    return await this._goto('frame.goto', url, options)
  }

  /** Sends `Frame.goto` under the navigation timeout, `waitUntil` defaulting to `load`. */
  async _goto(
    apiName: ApiName,
    url: string,
    options: GotoOptions,
  ): Promise<null> {
    const waitUntil = verifyLoadState(
      'waitUntil',
      options.waitUntil === undefined ? 'load' : options.waitUntil,
    )
    await this._connection.send(
      apiName,
      this._guid,
      'goto',
      pickParams(FRAME_GOTO_KEYS, {
        url,
        ...options,
        waitUntil,
      }) as FrameGotoParams,
      this._page._timeoutSettings.navigationTimeout(options),
    )
    return null
  }

  async title(): Promise<string> {
    return await this._title('frame.title')
  }

  /** Sends `Frame.title` with no timeout, as Playwright's `kNoTimeout`. */
  async _title(apiName: ApiName): Promise<string> {
    const result = (await this._connection.send(
      apiName,
      this._guid,
      'title',
      {},
      0,
    )) as FrameTitleResult
    return result.value
  }

  /** Not strict, as Playwright's `frame.textContent`. */
  async textContent(
    selector: string,
    options: TextContentOptions = {},
  ): Promise<string | null> {
    return await this._textContent('frame.textContent', selector, options)
  }

  async _textContent(
    apiName: ApiName,
    selector: string,
    options: TextContentOptions,
  ): Promise<string | null> {
    const result = (await this._connection.send(
      apiName,
      this._guid,
      'textContent',
      pickParams(FRAME_TEXT_CONTENT_KEYS, {
        selector,
        ...options,
      }) as FrameTextContentParams,
      this._page._timeoutSettings.timeout(options),
    )) as FrameTextContentResult
    return result.value === undefined ? null : result.value
  }

  async innerText(
    selector: string,
    options: InnerTextOptions = {},
  ): Promise<string> {
    return await this._innerText('frame.innerText', selector, options)
  }

  async _innerText(
    apiName: ApiName,
    selector: string,
    options: InnerTextOptions,
  ): Promise<string> {
    const result = (await this._connection.send(
      apiName,
      this._guid,
      'innerText',
      pickParams(FRAME_INNER_TEXT_KEYS, {
        selector,
        ...options,
      }) as FrameInnerTextParams,
      this._page._timeoutSettings.timeout(options ?? {}),
    )) as FrameInnerTextResult
    return result.value
  }

  async getAttribute(
    selector: string,
    name: string,
    options: GetAttributeOptions = {},
  ): Promise<string | null> {
    return await this._getAttribute(
      'frame.getAttribute',
      selector,
      name,
      options,
    )
  }

  async _getAttribute(
    apiName: ApiName,
    selector: string,
    name: string,
    options: GetAttributeOptions,
  ): Promise<string | null> {
    const result = (await this._connection.send(
      apiName,
      this._guid,
      'getAttribute',
      pickParams(FRAME_GET_ATTRIBUTE_KEYS, {
        selector,
        name,
        ...options,
      }) as FrameGetAttributeParams,
      this._page._timeoutSettings.timeout(options ?? {}),
    )) as FrameGetAttributeResult
    return result.value === undefined ? null : result.value
  }

  /** Sends `Frame.ariaSnapshot` for the whole frame: no selector, as `page.ariaSnapshot`. */
  async _ariaSnapshot(
    apiName: ApiName,
    options: AriaSnapshotOptions,
  ): Promise<string> {
    const result = (await this._connection.send(
      apiName,
      this._guid,
      'ariaSnapshot',
      pickParams(FRAME_ARIA_SNAPSHOT_KEYS, {
        mode: options.mode,
        depth: options.depth,
        boxes: options.boxes,
      }) as FrameAriaSnapshotParams,
      this._page._timeoutSettings.timeout(options),
    )) as FrameAriaSnapshotResult
    return result.snapshot
  }

  /** Not strict, as Playwright's `frame.click`. */
  async click(selector: string, options: ClickOptions = {}): Promise<void> {
    return await this._click('frame.click', selector, options)
  }

  /** Sends `Frame.click`. */
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
      this._page._timeoutSettings.timeout(options ?? {}),
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
      this._page._timeoutSettings.timeout(options ?? {}),
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
      this._page._timeoutSettings.timeout(options ?? {}),
    )
    if (result === undefined)
      throw new Error(
        'neo-sdk: Frame.inputValue contract error: missing result',
      )
    return result.value
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
      this._page._timeoutSettings.timeout(options ?? {}),
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
      this._page._timeoutSettings.timeout(options ?? {}),
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
      this._page._timeoutSettings.timeout(options ?? {}),
    )
    if (result === undefined)
      throw new Error(
        'neo-sdk: Frame.selectOption contract error: missing result',
      )
    return result.values
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
