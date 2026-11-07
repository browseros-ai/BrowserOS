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

import type { Connection } from './connection'
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

export class ElementHandle<T = unknown> extends JSHandle<T> {
  constructor(
    connection: Connection,
    guid: string,
    initializer: JSHandleInitializer,
  ) {
    super(connection, guid, initializer, 'elementHandle')
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

/** The `ElementHandle`s of a `querySelectorAll` answer, in order. */
export function elementsFrom(
  connection: Connection,
  created: CreatedHandle[],
): ElementHandle[] {
  return created
    .map((record) => ElementHandle.fromNullable(connection, record))
    .filter((handle): handle is ElementHandle => handle !== null)
}

handleFactory.element = (connection, guid, initializer) =>
  new ElementHandle(connection, guid, initializer)
