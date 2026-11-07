/**
 * Playwright: `client/consoleMessage.ts` `ConsoleMessage`
 *
 * One console message of a page, built from the `console` event Rust sends on the context
 * (`spec/playwright/browserContext.yml` `console`, the `ConsoleMessage` mixin) or from a
 * `consoleMessages()` record. Its `text` is the server's formula, computed in Rust from the
 * message's `RemoteObject`s (neo-runtime `console.rs`).
 *
 * Deviation: `client/consoleMessage.ts` `ConsoleMessage.args`: `return this._event.args.map(JSHandle.from);`
 * Playwright's server sends every argument as a handle with the message. Neo sends a message's
 * arguments by an id (`argsId`) and makes the run's handles of them when `args()` first reads
 * them (a getter; neo-runtime `tabs.rs` `console_args`), so a message nobody reads costs no
 * browser call and keeps its place among the events. Each handle lives in the frame that logged
 * the message, as Playwright's does. After the page closed, `args()` still answers with them, and
 * an object's handle then fails on use, as Playwright's does. A message read from
 * `consoleMessages()` kept no arguments: its `args()` throws an Error naming why.
 *
 * Left out: Node's `util.inspect.custom` hook, which QuickJS does not have.
 */

import type { PageConsoleMessagesResult } from '@browseros/neo-runtime-spec/serve'
import type { Connection } from './connection'
import { handleFrom, type JSHandle } from './js-handle'
import type { Page } from './page'
import { get } from './runtime-bridge'

/** The `ConsoleMessage` mixin as Rust sends it, its `args` empty; a `console` event's names its
 * arguments by `argsId` instead, when it has any. */
export type ConsoleMessagePayload =
  PageConsoleMessagesResult['messages'][number] & { argsId?: string }

/** Neo: why `args()` refuses a message read from `consoleMessages()`, which kept no arguments. */
const HISTORY_KEEPS_NO_ARGS =
  'Neo keeps no arguments of a message read from consoleMessages(); listen to "console" for them'

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
  /** The connection a `console` event's arguments become handles on; `null` for a message read
   * from `consoleMessages()`, which kept none. */
  readonly #connection: Connection | null
  #args: JSHandle[] | undefined

  /** `worker` is always `null`: Neo reports no worker's console yet. */
  constructor(
    event: ConsoleMessagePayload,
    page: Page | null,
    worker: null,
    connection: Connection | null = null,
  ) {
    this.#page = page
    this.#event = event
    this.#connection = connection
    void worker
  }

  /**
   * The message's arguments as the run's handles, the same objects at every read: values and
   * objects as `console.log` received them. A browser log entry has none, as in Playwright.
   */
  args(): JSHandle[] {
    if (!this.#args) {
      const connection = this.#connection
      if (!connection) throw new Error(HISTORY_KEEPS_NO_ARGS)
      const id = this.#event.argsId
      this.#args = id
        ? get(id, 'args').map((created) => handleFrom(connection, created))
        : []
    }
    return [...this.#args]
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
