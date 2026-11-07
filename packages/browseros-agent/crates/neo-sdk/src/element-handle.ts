/**
 * Playwright: `client/elementHandle.ts` `ElementHandle`
 *
 * Playwright's client `ElementHandle`: a `JSHandle` whose page object is a DOM node, which
 * `asElement()` returns itself for. It keeps only what works end to end today: everything a
 * `JSHandle` does (`evaluate`, `evaluateHandle`, `jsonValue`, `getProperty`, `dispose`,
 * `toString`), `asElement`, and the queries within it (`$`, `$$`, `$eval`, `$$eval`); its element
 * actions and reads (`click`, `boundingBox`, …) are separate ledger rows.
 *
 * `JSHandle`'s module builds these through `handleFactory`, set below, so the two files import
 * one way only.
 */

import type { Buffer } from './buffer'
import type { ApiName, Connection } from './connection'
import type { Frame } from './frame'
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
} from './js-handle'
import {
  elementScreenshotParams,
  resultBuffer,
  type ScreenshotOptions,
} from './screenshot'
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
    frame?: Frame,
  ) {
    super(connection, guid, initializer, 'elementHandle', frame)
  }

  /** Playwright's `ElementHandle.fromNullable`, over the record Rust created in `frame`. */
  static fromNullable(
    connection: Connection,
    created: CreatedHandle | undefined,
    frame: Frame | undefined,
  ): ElementHandle | null {
    if (!created) return null
    const handle = handleFrom(connection, created, frame)
    return handle instanceof ElementHandle ? handle : null
  }

  override asElement(): ElementHandle<T> {
    return this
  }

  async $(selector: string): Promise<ElementHandle | null> {
    const result = await this._connection.sendHandle(
      'elementHandle.$',
      this._guid,
      'querySelector',
      { selector },
      0,
    )
    return ElementHandle.fromNullable(
      this._connection,
      result.element,
      this._frame,
    )
  }

  async $$(selector: string): Promise<ElementHandle[]> {
    const result = await this._connection.sendHandle(
      'elementHandle.$$',
      this._guid,
      'querySelectorAll',
      { selector },
      0,
    )
    return elementsFrom(this._connection, result.elements, this._frame)
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

  /**
   * The element's image, captured where it is on the page, never a selector resolved again.
   * Playwright: `client/elementHandle.ts` `ElementHandle.screenshot`
   */
  async screenshot(options: ScreenshotOptions = {}): Promise<Buffer> {
    const result = await this._connection.sendHandle(
      'elementHandle.screenshot',
      this._guid,
      'screenshot',
      elementScreenshotParams('elementHandle.screenshot', options),
      this._timeout(options, 'elementHandle.screenshot'),
    )
    return resultBuffer(result.binary)
  }

  /**
   * The call's timeout and signal from its page's `TimeoutSettings`; a handle without a frame
   * gets fresh settings, as Playwright's `Frame._timeout` does without a page.
   * Playwright: `client/frame.ts` `Frame._timeout`: `const timeoutSettings = this._page?._timeoutSettings || new TimeoutSettings();`
   */
  _timeout(options: TimeoutInput, apiName: ApiName): TimeoutOptions {
    const settings =
      this._frame?._page._timeoutSettings ?? new TimeoutSettings()
    return settings.timeout(options, apiName)
  }
}

/** The `ElementHandle`s of a `querySelectorAll` answer in `frame`, in order. */
export function elementsFrom(
  connection: Connection,
  created: CreatedHandle[],
  frame: Frame | undefined,
): ElementHandle[] {
  return created
    .map((record) => ElementHandle.fromNullable(connection, record, frame))
    .filter((handle): handle is ElementHandle => handle !== null)
}

handleFactory.element = (connection, guid, initializer, frame) =>
  new ElementHandle(connection, guid, initializer, frame)
