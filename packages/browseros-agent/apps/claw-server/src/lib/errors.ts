/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Server-side error types and the JSON shape the routes return on
 * failure. The future UI consumes these via parseResponse: every
 * non-OK response carries `{ error: string }` and the client throws
 * a typed ApiError with `.status` and `.body` attached. Discriminated
 * `{ success: false }` unions are deliberately avoided so callers can
 * always trust the happy-path return type.
 */

export class HttpError extends Error {
  readonly status: number

  constructor(status: number, message: string) {
    super(message)
    this.status = status
    this.name = 'HttpError'
  }
}
