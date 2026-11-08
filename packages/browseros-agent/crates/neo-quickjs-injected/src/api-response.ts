/**
 * Playwright: `client/fetch.ts` `APIResponse`
 *
 * What `route.fetch` answers: the response the browser received for the routed request, its
 * body read whole (neo-runtime `routes.rs` `fetch`). Playwright's is an object of its
 * `APIRequestContext`, whose body the server keeps until `dispose()`; Neo's carries the body
 * itself, so `dispose()` only lets it go.
 */

import type { NameValue } from '@browseros/neo-runtime-spec/serve'
import { Buffer } from './buffer'
import type { NeoRouteFetchResult } from './neo-commands'
import { type Headers, RawHeaders } from './network'

/** Playwright: `client/fetch.ts` `APIResponse.body`: `throw new Error('Response has been disposed');` */
const DISPOSED = 'Response has been disposed'

export class APIResponse {
  readonly #url: string
  readonly #status: number
  readonly #statusText: string
  readonly #headers: RawHeaders
  #body: Buffer | null

  constructor(record: NeoRouteFetchResult) {
    this.#url = record.url
    this.#status = record.status
    this.#statusText = record.statusText
    this.#headers = new RawHeaders(record.headers)
    this.#body = Buffer.fromBinary(record.body)
  }

  ok(): boolean {
    return this.#status >= 200 && this.#status <= 299
  }

  url(): string {
    return this.#url
  }

  status(): number {
    return this.#status
  }

  statusText(): string {
    return this.#statusText
  }

  headers(): Headers {
    return this.#headers.headers()
  }

  headersArray(): NameValue[] {
    return this.#headers.headersArray()
  }

  async body(): Promise<Buffer> {
    if (!this.#body) throw new Error(DISPOSED)
    return this.#body
  }

  async text(): Promise<string> {
    return (await this.body()).toString('utf8')
  }

  async json(): Promise<unknown> {
    return JSON.parse(await this.text())
  }

  async dispose(): Promise<void> {
    this.#body = null
  }
}
