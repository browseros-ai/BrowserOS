/**
 * Playwright's client error classes (`client/errors.ts` at 26137face), with the same names and
 * `name` behaviour. Left out: the serialized-value branch of `parseError` (Neo's server only
 * ever replies with an Error).
 *
 * The one surprise is Playwright's own: `TargetClosedError` never sets `name`, so a page closed
 * mid-call surfaces as `e.name === 'Error'`. Scripts compare `e.name`, never the class, which
 * bundlers rename (`goldens/README.md`).
 */

export class PlaywrightError extends Error {
  log: string[] = []
}

export class TimeoutError extends PlaywrightError {
  constructor(message: string) {
    super(message)
    this.name = 'TimeoutError'
  }
}

/**
 * What a call whose `signal` aborted rejects with: `connection.ts` throws it before sending a
 * call whose signal already aborted, and rebuilds an aborted call's reply as it, both with the
 * signal's `reason` as `cause`. Scripts read `e.name === 'AbortError'`; Playwright's public
 * `errors` (`client/playwright.ts`) does not name the class either.
 */
export class AbortError extends PlaywrightError {
  constructor(message = 'The operation was aborted', options?: ErrorOptions) {
    super(message, options)
    this.name = 'AbortError'
  }
}

/** Not exported: Playwright's public `errors` (`client/playwright.ts`) names only
 * `TimeoutError`, and a script never sees this class's name (it reads `e.name === 'Error'`). */
class TargetClosedError extends PlaywrightError {
  constructor(cause?: string) {
    super(cause || 'Target page, context or browser has been closed')
  }
}

/** Playwright's `TargetClosedError` for a page that closed under a wait the SDK runs itself
 * (`response.finished()`), with its default message. */
export function targetClosedError(): PlaywrightError {
  return new TargetClosedError()
}

/** Playwright's `isTargetClosedError`: whether a call failed because its page was already
 * closed. It tests the class, as Playwright's does: the `name` reads `'Error'` and the message
 * may carry a close reason, so neither tells this error apart. */
export function isTargetClosedError(error: unknown): boolean {
  return error instanceof TargetClosedError
}

/** Playwright's `parseError`: the class by wire `name`; any other name is copied onto a
 * `PlaywrightError`. The stack is the serialized one, or empty, as Playwright sets it. */
export function parseError(error: {
  name: string
  message: string
  stack?: string
}): PlaywrightError {
  let e: PlaywrightError
  if (error.name === 'TimeoutError') e = new TimeoutError(error.message)
  else if (error.name === 'TargetClosedError')
    e = new TargetClosedError(error.message)
  else if (error.name === 'AbortError') e = new AbortError(error.message)
  else
    e = Object.assign(new PlaywrightError(error.message), {
      name: error.name,
    })
  if (error.stack !== undefined) e.stack = error.stack
  return e
}

/**
 * Playwright's `parseError` of a whole `SerializedError`, the page errors Rust sends
 * (`pageError` events, `pageErrors()`): always its `error` branch, since Rust builds an `Error`
 * from every exception as Playwright's server does (`exceptionToError`).
 */
export function parseSerializedError(error: {
  error?: { name: string; message: string; stack?: string }
}): PlaywrightError {
  if (!error.error)
    throw new Error('Serialized error must have either an error or a value')
  return parseError({ ...error.error, stack: error.error.stack || '' })
}
