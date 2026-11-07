/**
 * Playwright: `client/webError.ts` `WebError`
 *
 * An uncaught exception of a page, as the context's `weberror` event hands it out: the page,
 * the `Error` the page's `pageerror` event also receives, and where it was thrown.
 */

import type { Page } from './page'

/** `pageError`'s location (`spec/playwright/browserContext.yml`): 0-based, as CDP reports it. */
export interface WebErrorLocation {
  url: string
  line: number
  column: number
}

export class WebError {
  readonly #page: Page | null
  readonly #error: Error
  readonly #location: WebErrorLocation

  constructor(page: Page | null, error: Error, location: WebErrorLocation) {
    this.#page = page
    this.#error = error
    this.#location = location
  }

  page(): Page | null {
    return this.#page
  }

  error(): Error {
    return this.#error
  }

  location(): WebErrorLocation {
    return this.#location
  }
}
