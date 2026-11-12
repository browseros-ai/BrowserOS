/**
 * Upstream's WebStorage (client/webStorage.ts at v1.64.1): `page.localStorage` and
 * `page.sessionStorage`. Each method is one op on the page's main frame, without a timeout,
 * as upstream's calls are (`kNoTimeout`): `webStorage.read` for `items` and `getItem`,
 * `webStorage.write` for the rest, so the trace says which calls changed the page.
 */
import { captureSite } from './call-site'
import { clientError } from './errors'
import { send } from './generated'
import type { Page } from './page'

type StorageKind = 'local' | 'session'

export class WebStorage {
  readonly #page: Page
  readonly #kind: StorageKind

  constructor(page: Page, kind: StorageKind) {
    this.#page = page
    this.#kind = kind
  }

  async items(): Promise<{ name: string; value: string }[]> {
    const site = captureSite()
    const api = 'webStorage.items'
    const items = await send['webStorage.read'](
      site,
      { api, timeoutMs: 0 },
      { page: await this.#page._brand(api), kind: this.#kind, action: 'items' },
    )
    return Array.isArray(items) ? items : []
  }

  async getItem(name: string): Promise<string | null> {
    const site = captureSite()
    const api = 'webStorage.getItem'
    const value = await send['webStorage.read'](
      site,
      { api, timeoutMs: 0 },
      {
        page: await this.#page._brand(api),
        kind: this.#kind,
        action: 'getItem',
        name: text(api, 'name', name),
      },
    )
    return typeof value === 'string' ? value : null
  }

  async setItem(name: string, value: string): Promise<void> {
    const site = captureSite()
    const api = 'webStorage.setItem'
    await send['webStorage.write'](
      site,
      { api, timeoutMs: 0 },
      {
        page: await this.#page._brand(api),
        kind: this.#kind,
        action: 'setItem',
        name: text(api, 'name', name),
        value: text(api, 'value', value),
      },
    )
  }

  async removeItem(name: string): Promise<void> {
    const site = captureSite()
    const api = 'webStorage.removeItem'
    await send['webStorage.write'](
      site,
      { api, timeoutMs: 0 },
      {
        page: await this.#page._brand(api),
        kind: this.#kind,
        action: 'removeItem',
        name: text(api, 'name', name),
      },
    )
  }

  async clear(): Promise<void> {
    const site = captureSite()
    const api = 'webStorage.clear'
    await send['webStorage.write'](
      site,
      { api, timeoutMs: 0 },
      { page: await this.#page._brand(api), kind: this.#kind, action: 'clear' },
    )
  }
}

/** Upstream's protocol validator requires strings here, and says which field was not one. */
function text(api: string, field: string, value: unknown): string {
  if (typeof value !== 'string')
    throw clientError(api, `${field}: expected string, got ${typeof value}`)
  return value
}
