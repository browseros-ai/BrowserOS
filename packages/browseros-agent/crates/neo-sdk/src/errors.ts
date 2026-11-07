/**
 * Playwright's client error classes (`client/errors.ts` at 26137face), with the same names and
 * `name` behaviour. Left out: `AbortError` (Neo takes no `signal`) and the serialized-value
 * branch of `parseError` (Neo's server only ever replies with an Error).
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
 * `PlaywrightError`. */
export function parseError(error: {
  name: string
  message: string
}): PlaywrightError {
  if (error.name === 'TimeoutError') return new TimeoutError(error.message)
  if (error.name === 'TargetClosedError')
    return new TargetClosedError(error.message)
  return Object.assign(new PlaywrightError(error.message), {
    name: error.name,
  })
}
