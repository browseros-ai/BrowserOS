/**
 * Playwright: `client/elementHandle.ts` `ElementHandle`
 *
 * Playwright's client `ElementHandle`: a `JSHandle` whose page object is a DOM node, which
 * `asElement()` returns itself for. It keeps only what works end to end today: everything a
 * `JSHandle` does (`evaluate`, `evaluateHandle`, `jsonValue`, `getProperty`, `dispose`,
 * `toString`), `asElement`, the queries within it (`$`, `$$`, `$eval`, `$$eval`), and the reads
 * on its own element: `textContent`, `innerText`, `innerHTML`, `getAttribute`, `inputValue`, the
 * `is*` states, `boundingBox`, `ownerFrame` and `contentFrame` (neo-runtime `element_handle.rs`).
 * Its actions (`click`, `fill`, …) are separate ledger rows.
 *
 * `JSHandle`'s module builds these through `handleFactory`, set below, so the two files import
 * one way only.
 */

import type { Rect } from '@browseros/neo-runtime-spec/serve'
import type { Connection } from './connection'
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
import type { CreatedFrame } from './runtime-bridge'

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

handleFactory.element = (connection, guid, initializer) =>
  new ElementHandle(connection, guid, initializer)
