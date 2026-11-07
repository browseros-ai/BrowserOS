/**
 * Playwright: `client/consoleMessage.ts` `ConsoleMessage`
 *
 * One console message of a page, built from the `console` event Rust sends on the context
 * (`spec/playwright/browserContext.yml` `console`, the `ConsoleMessage` mixin) or from a
 * `consoleMessages()` record. Its `text` is the server's formula, computed in Rust from the
 * message's `RemoteObject`s (neo-runtime `console.rs`).
 *
 * Deviation: `client/consoleMessage.ts` `ConsoleMessage.args`: `return this._event.args.map(JSHandle.from);`
 * Neo has no `JSHandle` for a console argument yet, so a message has no `args()`; a script that
 * calls it gets a TypeError rather than a list of stand-ins.
 *
 * Left out: Node's `util.inspect.custom` hook, which QuickJS does not have.
 */

import type { PageConsoleMessagesResult } from '@browseros/neo-runtime-spec/serve'
import type { Page } from './page'

/** The `ConsoleMessage` mixin as Rust sends it: `args` always empty. */
export type ConsoleMessagePayload =
  PageConsoleMessagesResult['messages'][number]

/** What `location()` returns: Playwright's 0-based position, under both names. */
export interface ConsoleMessageLocation {
  url: string
  line: number
  column: number
  lineNumber: number
  columnNumber: number
}

export class ConsoleMessage {
  readonly #page: Page | null
  readonly #event: ConsoleMessagePayload

  /** `worker` is always `null`: Neo reports no worker's console yet. */
  constructor(event: ConsoleMessagePayload, page: Page | null, worker: null) {
    this.#page = page
    this.#event = event
    void worker
  }

  worker(): null {
    return null
  }

  page(): Page | null {
    return this.#page
  }

  type(): string {
    return this.#event.type
  }

  text(): string {
    return this.#event.text
  }

  location(): ConsoleMessageLocation {
    const { url, lineNumber, columnNumber } = this.#event.location
    return {
      url,
      line: lineNumber,
      column: columnNumber,
      lineNumber,
      columnNumber,
    }
  }

  timestamp(): number {
    return this.#event.timestamp
  }
}
