/**
 * Playwright: `client/dialog.ts` `Dialog`
 *
 * A JavaScript dialog (`alert`, `confirm`, `prompt`, `beforeunload`) the run's `dialog` event
 * handed out. Rust mints its guid (`dialog@<targetId>:<n>`) and serves `accept` and `dismiss`
 * as `Page.handleJavaScriptDialog` on the dialog's tab; a second answer rejects. A dialog
 * nobody answers blocks its page, as in Playwright, until the run ends, when Rust answers it
 * with the default (neo-runtime `dialogs.rs`).
 *
 * Playwright's `Dialog` is a `ChannelOwner`, so an `EventEmitter` that emits nothing; Neo's is a
 * plain emitter for the same reason.
 */

import type { DialogInitializer } from '@browseros/neo-playwright-protocol/serve'
import type { Connection } from './connection'
import { isTargetClosedError } from './errors'
import { EventEmitter } from './events'
import type { Page } from './page'

export class Dialog extends EventEmitter {
  readonly #guid: string
  readonly #connection: Connection
  readonly #initializer: DialogInitializer
  readonly #page: Page | null

  constructor(
    guid: string,
    initializer: DialogInitializer,
    page: Page | null,
    connection: Connection,
  ) {
    super()
    this.#guid = guid
    this.#initializer = initializer
    this.#page = page
    this.#connection = connection
  }

  page(): Page | null {
    return this.#page
  }

  type(): string {
    return this.#initializer.type
  }

  message(): string {
    return this.#initializer.message
  }

  defaultValue(): string {
    return this.#initializer.defaultValue
  }

  async accept(promptText?: string): Promise<void> {
    // Playwright's validator lets only a string or `undefined` through (`tOptional(tString)`).
    if (promptText !== undefined && typeof promptText !== 'string')
      throw new Error(
        `promptText: expected string, got ${promptText === null ? 'object' : typeof promptText}`,
      )
    await this.#connection.sendDialog(
      'dialog.accept',
      this.#guid,
      'accept',
      promptText === undefined ? {} : { promptText },
      0,
    )
  }

  async dismiss(): Promise<void> {
    try {
      await this.#connection.sendDialog(
        'dialog.dismiss',
        this.#guid,
        'dismiss',
        {},
        0,
      )
    } catch (e) {
      if (isTargetClosedError(e)) return
      throw e
    }
  }

  /** The no-listener answer, sent as Playwright's context sends it: failures ignored. */
  _answerDefault(): void {
    const answer =
      this.type() === 'beforeunload'
        ? this.#connection.sendDialog(
            'dialog.accept',
            this.#guid,
            'accept',
            {},
            0,
          )
        : this.#connection.sendDialog(
            'dialog.dismiss',
            this.#guid,
            'dismiss',
            {},
            0,
          )
    answer.catch(() => {})
  }

  /** Playwright's `ChannelOwner.toJSON`: the dialog's type and guid only. */
  toJSON(): { _type: 'Dialog'; _guid: string } {
    return { _type: 'Dialog', _guid: this.#guid }
  }
}
