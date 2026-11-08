/**
 * Playwright: `client/webStorage.ts` `WebStorage`
 *
 * `page.localStorage` and `page.sessionStorage`: the page's main frame's storage. Each method is
 * one Page call, sent with no timeout and no signal as Playwright's are (`kNoTimeout`), which
 * neo-runtime serves with glue in the main frame's utility world (`web_storage.rs`).
 */

import type { Connection } from './connection'

/** Neo: one storage item, as `WebStorage.items` answers it (`{ name, value }`). */
export type WebStorageItem = { name: string; value: string }

/** Playwright: `client/webStorage.ts` `WebStorage` */
export class WebStorage {
  readonly #connection: Connection
  readonly #page: string
  readonly #kind: 'local' | 'session'

  /** `page` is the guid of the page whose main frame's storage this is. */
  constructor(connection: Connection, page: string, kind: 'local' | 'session') {
    this.#connection = connection
    this.#page = page
    this.#kind = kind
  }

  /** Playwright: `client/webStorage.ts` `WebStorage.items` */
  async items(): Promise<WebStorageItem[]> {
    const { items } = await this.#connection.sendPage(
      'webStorage.items',
      this.#page,
      'webStorageItems',
      { kind: this.#kind },
      0,
    )
    return items
  }

  /** Playwright: `client/webStorage.ts` `WebStorage.getItem` */
  async getItem(name: string): Promise<string | null> {
    const { value } = await this.#connection.sendPage(
      'webStorage.getItem',
      this.#page,
      'webStorageGetItem',
      { kind: this.#kind, name },
      0,
    )
    return value ?? null
  }

  /** Playwright: `client/webStorage.ts` `WebStorage.setItem` */
  async setItem(name: string, value: string): Promise<void> {
    await this.#connection.sendPage(
      'webStorage.setItem',
      this.#page,
      'webStorageSetItem',
      { kind: this.#kind, name, value },
      0,
    )
  }

  /** Playwright: `client/webStorage.ts` `WebStorage.removeItem` */
  async removeItem(name: string): Promise<void> {
    await this.#connection.sendPage(
      'webStorage.removeItem',
      this.#page,
      'webStorageRemoveItem',
      { kind: this.#kind, name },
      0,
    )
  }

  /** Playwright: `client/webStorage.ts` `WebStorage.clear` */
  async clear(): Promise<void> {
    await this.#connection.sendPage(
      'webStorage.clear',
      this.#page,
      'webStorageClear',
      { kind: this.#kind },
      0,
    )
  }
}
