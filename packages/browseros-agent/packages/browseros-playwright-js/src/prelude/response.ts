/**
 * What `page.goto` and the history methods resolve with. Neo reads the committed response's
 * status from the new document's navigation timing entry and never enables the Network domain,
 * so headers are empty and bodies cannot be read (deviations recorded in design.md). Rust
 * sends the status, its reason phrase and `ok`.
 */
import type { Frame } from './frame'
import { markHandle } from './options'
import type { ResponseInfo } from './wire'

export class Response {
  readonly #frameValue: Frame
  readonly #info: ResponseInfo

  constructor(frame: Frame, info: ResponseInfo) {
    this.#frameValue = frame
    this.#info = info
    markHandle(this)
  }

  url(): string {
    return this.#info.url
  }

  status(): number {
    return this.#info.status
  }

  statusText(): string {
    return this.#info.statusText
  }

  ok(): boolean {
    return this.#info.ok
  }

  frame(): Frame {
    return this.#frameValue
  }

  headers(): Record<string, string> {
    return {}
  }

  async allHeaders(): Promise<Record<string, string>> {
    return {}
  }

  async headersArray(): Promise<{ name: string; value: string }[]> {
    return []
  }

  async headerValue(_name: string): Promise<string | null> {
    return null
  }

  async headerValues(_name: string): Promise<string[]> {
    return []
  }

  async finished(): Promise<null> {
    return null
  }

  fromServiceWorker(): boolean {
    return false
  }

  toJSON(): ResponseInfo {
    return this.#info
  }
}
