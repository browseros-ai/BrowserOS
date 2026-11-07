/**
 * Playwright: `client/elementHandle.ts` `ElementHandle`
 *
 * Playwright's client `ElementHandle`: a `JSHandle` whose page object is a DOM node, which
 * `asElement()` returns itself for. It keeps only what works end to end today: everything a
 * `JSHandle` does (`evaluate`, `evaluateHandle`, `jsonValue`, `getProperty`, `dispose`,
 * `toString`), `asElement`, the queries within it (`$`, `$$`, `$eval`, `$$eval`), and the reads
 * on its own element: `textContent`, `innerText`, `innerHTML`, `getAttribute`, `inputValue`, the
 * `is*` states, `boundingBox`, `ownerFrame` and `contentFrame`, and the actions on it (`click`,
 * `dblclick`, `hover`, `tap`, `check`, `uncheck`, `setChecked`, `fill`, `press`, `type`,
 * `selectOption`, `selectText`, `focus`, `scrollIntoViewIfNeeded`), which neo-runtime runs on the
 * handle's own element with the frame actions' per-element steps (`element_handle.rs`). An
 * action's timeout is its page's, as Playwright's client reads it through the handle's frame.
 *
 * `JSHandle`'s module builds these through `handleFactory`, set below, so the two files import
 * one way only.
 */

import type {
  ElementHandleCheckParams,
  ElementHandleClickParams,
  ElementHandleDblclickParams,
  ElementHandleFillParams,
  ElementHandleHoverParams,
  ElementHandlePressParams,
  ElementHandleSelectOptionParams,
  ElementHandleSelectTextParams,
  ElementHandleTapParams,
  ElementHandleTypeParams,
  ElementHandleUncheckParams,
  ElementHandleWaitForElementStateParams,
  ElementHandleWaitForSelectorParams,
  Rect,
} from '@browseros/neo-runtime-spec/serve'
import type { ApiName, Connection } from './connection'
import type {
  CheckOptions,
  ClickOptions,
  DblclickOptions,
  FillOptions,
  Frame,
  HoverOptions,
  PressOptions,
  SelectOptionOptions,
  SelectTextOptions,
  TapOptions,
  TypeOptions,
  UncheckOptions,
  WaitForSelectorOptions,
} from './frame'
import {
  type CreatedHandle,
  evaluateParams,
  handleFactory,
  handleFrom,
  JSHandle,
  type JSHandleInitializer,
  type PageElement,
  type PageFunctionOn,
  parseResult,
  serializeArgument,
} from './js-handle'
import {
  ELEMENT_HANDLE_CHECK_KEYS,
  ELEMENT_HANDLE_CLICK_KEYS,
  ELEMENT_HANDLE_DBLCLICK_KEYS,
  ELEMENT_HANDLE_FILL_KEYS,
  ELEMENT_HANDLE_HOVER_KEYS,
  ELEMENT_HANDLE_PRESS_KEYS,
  ELEMENT_HANDLE_SELECT_OPTION_KEYS,
  ELEMENT_HANDLE_SELECT_TEXT_KEYS,
  ELEMENT_HANDLE_TAP_KEYS,
  ELEMENT_HANDLE_TYPE_KEYS,
  ELEMENT_HANDLE_UNCHECK_KEYS,
  ELEMENT_HANDLE_WAIT_FOR_SELECTOR_KEYS,
  pickParams,
} from './protocol'
import type { CreatedFrame } from './runtime-bridge'
import {
  convertSelectOptionValues,
  type SelectOptionValues,
} from './select-option'
import {
  type TimeoutInput,
  type TimeoutOptions,
  TimeoutSettings,
} from './timeout-settings'

export class ElementHandle<T = unknown> extends JSHandle<T> {
  constructor(
    connection: Connection,
    guid: string,
    initializer: JSHandleInitializer,
    frameGuid?: string,
  ) {
    super(connection, guid, initializer, 'elementHandle', frameGuid)
  }

  /** The timeout of an action: its page's, through the frame the handle's record names.
   * Playwright: `client/frame.ts` `Frame._timeout`: `const timeoutSettings = this._page?._timeoutSettings || new TimeoutSettings();` */
  _timeout(options: TimeoutInput, apiName: ApiName): TimeoutOptions {
    const timeoutSettings = this._frameGuid
      ? this._connection._pageFor(this._frameGuid)._timeoutSettings
      : new TimeoutSettings()
    return timeoutSettings.timeout(options, apiName)
  }

  async scrollIntoViewIfNeeded(options: TimeoutInput = {}): Promise<void> {
    const apiName = 'elementHandle.scrollIntoViewIfNeeded'
    await this._connection.sendElement(
      apiName,
      this._guid,
      'scrollIntoViewIfNeeded',
      {},
      this._timeout(options, apiName),
    )
  }

  async hover(options: HoverOptions & TimeoutInput = {}): Promise<void> {
    const apiName = 'elementHandle.hover'
    await this._connection.sendElement(
      apiName,
      this._guid,
      'hover',
      pickParams(
        ELEMENT_HANDLE_HOVER_KEYS,
        options,
      ) as ElementHandleHoverParams,
      this._timeout(options, apiName),
    )
  }

  async click(options: ClickOptions & TimeoutInput = {}): Promise<void> {
    const apiName = 'elementHandle.click'
    await this._connection.sendElement(
      apiName,
      this._guid,
      'click',
      pickParams(
        ELEMENT_HANDLE_CLICK_KEYS,
        options,
      ) as ElementHandleClickParams,
      this._timeout(options, apiName),
    )
  }

  async dblclick(options: DblclickOptions & TimeoutInput = {}): Promise<void> {
    const apiName = 'elementHandle.dblclick'
    await this._connection.sendElement(
      apiName,
      this._guid,
      'dblclick',
      pickParams(
        ELEMENT_HANDLE_DBLCLICK_KEYS,
        options,
      ) as ElementHandleDblclickParams,
      this._timeout(options, apiName),
    )
  }

  async tap(options: TapOptions & TimeoutInput = {}): Promise<void> {
    const apiName = 'elementHandle.tap'
    await this._connection.sendElement(
      apiName,
      this._guid,
      'tap',
      pickParams(ELEMENT_HANDLE_TAP_KEYS, options) as ElementHandleTapParams,
      this._timeout(options, apiName),
    )
  }

  /** Playwright: `client/elementHandle.ts` `ElementHandle.selectOption`: `const result = await this._elementChannel.selectOption({ ...convertSelectOptionValues(values), ...options }, this._frame._timeout(options));` */
  async selectOption(
    values: SelectOptionValues,
    options: SelectOptionOptions & TimeoutInput = {},
  ): Promise<string[]> {
    const apiName = 'elementHandle.selectOption'
    const result = await this._connection.sendElement(
      apiName,
      this._guid,
      'selectOption',
      pickParams(ELEMENT_HANDLE_SELECT_OPTION_KEYS, {
        ...convertSelectOptionValues(values),
        ...options,
      }) as ElementHandleSelectOptionParams,
      this._timeout(options, apiName),
    )
    return result.values
  }

  async fill(
    value: string,
    options: FillOptions & TimeoutInput = {},
  ): Promise<void> {
    const apiName = 'elementHandle.fill'
    await this._connection.sendElement(
      apiName,
      this._guid,
      'fill',
      pickParams(ELEMENT_HANDLE_FILL_KEYS, {
        value,
        ...options,
      }) as ElementHandleFillParams,
      this._timeout(options, apiName),
    )
  }

  async selectText(
    options: SelectTextOptions & TimeoutInput = {},
  ): Promise<void> {
    const apiName = 'elementHandle.selectText'
    await this._connection.sendElement(
      apiName,
      this._guid,
      'selectText',
      pickParams(
        ELEMENT_HANDLE_SELECT_TEXT_KEYS,
        options,
      ) as ElementHandleSelectTextParams,
      this._timeout(options, apiName),
    )
  }

  /** Playwright: `client/elementHandle.ts` `ElementHandle.focus`: `await this._elementChannel.focus({}, kNoTimeout);` */
  async focus(): Promise<void> {
    await this._connection.sendElement(
      'elementHandle.focus',
      this._guid,
      'focus',
      {},
      0,
    )
  }

  async type(
    text: string,
    options: TypeOptions & TimeoutInput = {},
  ): Promise<void> {
    const apiName = 'elementHandle.type'
    await this._connection.sendElement(
      apiName,
      this._guid,
      'type',
      pickParams(ELEMENT_HANDLE_TYPE_KEYS, {
        text,
        ...options,
      }) as ElementHandleTypeParams,
      this._timeout(options, apiName),
    )
  }

  async press(
    key: string,
    options: PressOptions & TimeoutInput = {},
  ): Promise<void> {
    const apiName = 'elementHandle.press'
    await this._connection.sendElement(
      apiName,
      this._guid,
      'press',
      pickParams(ELEMENT_HANDLE_PRESS_KEYS, {
        key,
        ...options,
      }) as ElementHandlePressParams,
      this._timeout(options, apiName),
    )
  }

  async check(options: CheckOptions & TimeoutInput = {}): Promise<void> {
    const apiName = 'elementHandle.check'
    await this._connection.sendElement(
      apiName,
      this._guid,
      'check',
      pickParams(
        ELEMENT_HANDLE_CHECK_KEYS,
        options,
      ) as ElementHandleCheckParams,
      this._timeout(options, apiName),
    )
  }

  async uncheck(options: UncheckOptions & TimeoutInput = {}): Promise<void> {
    const apiName = 'elementHandle.uncheck'
    await this._connection.sendElement(
      apiName,
      this._guid,
      'uncheck',
      pickParams(
        ELEMENT_HANDLE_UNCHECK_KEYS,
        options,
      ) as ElementHandleUncheckParams,
      this._timeout(options, apiName),
    )
  }

  async waitForElementState(
    state: ElementHandleWaitForElementStateParams['state'],
    options: TimeoutInput = {},
  ): Promise<void> {
    const apiName = 'elementHandle.waitForElementState'
    await this._connection.sendElement(
      apiName,
      this._guid,
      'waitForElementState',
      { state },
      this._timeout(options, apiName),
    )
  }

  /** Playwright: `client/elementHandle.ts` `ElementHandle.waitForSelector`: `return ElementHandle.fromNullable(result.element) as ElementHandle<SVGElement | HTMLElement> | null;` */
  async waitForSelector(
    selector: string,
    options: WaitForSelectorOptions & TimeoutInput = {},
  ): Promise<ElementHandle | null> {
    const apiName = 'elementHandle.waitForSelector'
    const result = await this._connection.sendElement(
      apiName,
      this._guid,
      'waitForSelector',
      pickParams(ELEMENT_HANDLE_WAIT_FOR_SELECTOR_KEYS, {
        selector,
        ...options,
      }) as ElementHandleWaitForSelectorParams,
      this._timeout(options, apiName),
    )
    // Neo answers the element's handle record, as `$` does, where the wire type names a channel.
    const element = (result as unknown as { element?: CreatedHandle }).element
    return ElementHandle.fromNullable(this._connection, element)
  }

  /** Playwright: `client/elementHandle.ts` `ElementHandle.setChecked`: `if (checked)` */
  async setChecked(
    checked: boolean,
    options?: CheckOptions & TimeoutInput,
  ): Promise<void> {
    if (checked) await this.check(options)
    else await this.uncheck(options)
  }

  /** Playwright's `ElementHandle.fromNullable`, over the record Rust created. */
  static fromNullable(
    connection: Connection,
    created: CreatedHandle | undefined,
  ): ElementHandle | null {
    if (!created) return null
    const handle = handleFrom(connection, created)
    return handle instanceof ElementHandle ? handle : null
  }

  override asElement(): ElementHandle<T> {
    return this
  }

  /** Playwright: `client/elementHandle.ts` `ElementHandle.ownerFrame` */
  async ownerFrame(): Promise<Frame | null> {
    const result = await this._connection.sendElement(
      'elementHandle.ownerFrame',
      this._guid,
      'ownerFrame',
      {},
      0,
    )
    return frameFrom(this._connection, result.frame)
  }

  /** Playwright: `client/elementHandle.ts` `ElementHandle.contentFrame` */
  async contentFrame(): Promise<Frame | null> {
    const result = await this._connection.sendElement(
      'elementHandle.contentFrame',
      this._guid,
      'contentFrame',
      {},
      0,
    )
    return frameFrom(this._connection, result.frame)
  }

  /** Playwright: `client/elementHandle.ts` `ElementHandle.getAttribute`: `return value === undefined ? null : value;` */
  async getAttribute(name: string): Promise<string | null> {
    const value = (
      await this._connection.sendElement(
        'elementHandle.getAttribute',
        this._guid,
        'getAttribute',
        { name },
        0,
      )
    ).value
    return value === undefined ? null : value
  }

  async inputValue(): Promise<string> {
    return (
      await this._connection.sendElement(
        'elementHandle.inputValue',
        this._guid,
        'inputValue',
        {},
        0,
      )
    ).value
  }

  async textContent(): Promise<string | null> {
    const value = (
      await this._connection.sendElement(
        'elementHandle.textContent',
        this._guid,
        'textContent',
        {},
        0,
      )
    ).value
    return value === undefined ? null : value
  }

  async innerText(): Promise<string> {
    return (
      await this._connection.sendElement(
        'elementHandle.innerText',
        this._guid,
        'innerText',
        {},
        0,
      )
    ).value
  }

  async innerHTML(): Promise<string> {
    return (
      await this._connection.sendElement(
        'elementHandle.innerHTML',
        this._guid,
        'innerHTML',
        {},
        0,
      )
    ).value
  }

  async isChecked(): Promise<boolean> {
    return (
      await this._connection.sendElement(
        'elementHandle.isChecked',
        this._guid,
        'isChecked',
        {},
        0,
      )
    ).value
  }

  async isDisabled(): Promise<boolean> {
    return (
      await this._connection.sendElement(
        'elementHandle.isDisabled',
        this._guid,
        'isDisabled',
        {},
        0,
      )
    ).value
  }

  async isEditable(): Promise<boolean> {
    return (
      await this._connection.sendElement(
        'elementHandle.isEditable',
        this._guid,
        'isEditable',
        {},
        0,
      )
    ).value
  }

  async isEnabled(): Promise<boolean> {
    return (
      await this._connection.sendElement(
        'elementHandle.isEnabled',
        this._guid,
        'isEnabled',
        {},
        0,
      )
    ).value
  }

  async isHidden(): Promise<boolean> {
    return (
      await this._connection.sendElement(
        'elementHandle.isHidden',
        this._guid,
        'isHidden',
        {},
        0,
      )
    ).value
  }

  async isVisible(): Promise<boolean> {
    return (
      await this._connection.sendElement(
        'elementHandle.isVisible',
        this._guid,
        'isVisible',
        {},
        0,
      )
    ).value
  }

  /** Playwright: `client/elementHandle.ts` `ElementHandle.boundingBox`: `return value === undefined ? null : value;` */
  async boundingBox(): Promise<Rect | null> {
    const value = (
      await this._connection.sendElement(
        'elementHandle.boundingBox',
        this._guid,
        'boundingBox',
        {},
        0,
      )
    ).value
    return value === undefined ? null : value
  }

  async $(selector: string): Promise<ElementHandle | null> {
    const result = await this._connection.sendHandle(
      'elementHandle.$',
      this._guid,
      'querySelector',
      { selector },
      0,
    )
    return ElementHandle.fromNullable(this._connection, result.element)
  }

  async $$(selector: string): Promise<ElementHandle[]> {
    const result = await this._connection.sendHandle(
      'elementHandle.$$',
      this._guid,
      'querySelectorAll',
      { selector },
      0,
    )
    return elementsFrom(this._connection, result.elements)
  }

  async $eval<R, Arg>(
    selector: string,
    pageFunction: PageFunctionOn<PageElement, Arg, R>,
    arg?: Arg,
  ): Promise<R> {
    const params = evaluateParams(
      'elementHandle.$eval',
      pageFunction,
      arg,
      undefined,
    )
    const result = await this._connection.sendHandle(
      'elementHandle.$eval',
      this._guid,
      'evalOnSelector',
      { selector, ...params },
      0,
    )
    return parseResult(result.value) as R
  }

  async $$eval<R, Arg>(
    selector: string,
    pageFunction: PageFunctionOn<PageElement[], Arg, R>,
    arg?: Arg,
  ): Promise<R> {
    const params = evaluateParams(
      'elementHandle.$$eval',
      pageFunction,
      arg,
      undefined,
    )
    const result = await this._connection.sendHandle(
      'elementHandle.$$eval',
      this._guid,
      'evalOnSelectorAll',
      { selector, ...params },
      0,
    )
    return parseResult(result.value) as R
  }

  /** Playwright's `elementHandle.dispatchEvent`: a `type` event on this element, built from
   * `eventInit` as Playwright builds it, in the page's own world, so the page's listeners see
   * the event's objects (a `CustomEvent`'s `detail`, a `DataTransfer` handle) as its own. */
  async dispatchEvent(type: string, eventInit: object = {}): Promise<void> {
    await this._connection.sendHandle(
      'elementHandle.dispatchEvent',
      this._guid,
      'dispatchEvent',
      { type, eventInit: serializeArgument(eventInit) },
      0,
    )
  }
}

/** The `Frame` object of the frame record an `ownerFrame` or `contentFrame` answer holds; Neo's
 * record carries the frame's initializer beside Playwright's `{ guid }`. */
function frameFrom(connection: Connection, record: unknown): Frame | null {
  return record ? connection._frameFor(record as CreatedFrame) : null
}

/** The `ElementHandle`s of a `querySelectorAll` answer, in order. */
export function elementsFrom(
  connection: Connection,
  created: CreatedHandle[],
): ElementHandle[] {
  return created
    .map((record) => ElementHandle.fromNullable(connection, record))
    .filter((handle): handle is ElementHandle => handle !== null)
}

handleFactory.element = (connection, guid, initializer, frameGuid) =>
  new ElementHandle(connection, guid, initializer, frameGuid)
