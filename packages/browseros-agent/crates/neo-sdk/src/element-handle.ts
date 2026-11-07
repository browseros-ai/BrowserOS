/**
 * Playwright: `client/elementHandle.ts` `ElementHandle`
 *
 * Playwright's client `ElementHandle`: a `JSHandle` whose page object is a DOM node, which
 * `asElement()` returns itself for. It keeps only what works end to end today: everything a
 * `JSHandle` does (`evaluate`, `evaluateHandle`, `jsonValue`, `dispose`, `toString`) plus
 * `asElement`; its element methods (`click`, `$`, `boundingBox`, …) are separate ledger rows.
 *
 * `JSHandle`'s module builds these through `handleFactory`, set below, so the two files import
 * one way only.
 */

import type { Connection } from './connection'
import {
  type CreatedHandle,
  handleFactory,
  handleFrom,
  JSHandle,
  type JSHandleInitializer,
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
}

handleFactory.element = (connection, guid, initializer) =>
  new ElementHandle(connection, guid, initializer)
