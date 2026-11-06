/**
 * Errors as a script sees them: Playwright's names and first lines, carrying claw-sdk's fields.
 *
 * Two sources:
 * - an op failed: the bootstrap rejects `__neo.invoke` with its NeoError (a TimeoutError when
 *   the op timed out) holding the envelope's code, owner, delivery, retryable and operationId.
 *   `shapeBridgeError` reshapes that object in place, expect's failures included: the
 *   bootstrap knows its NeoErrors by identity (a WeakMap), so only that object settles an
 *   uncaught failure as the op's own, with the owner and delivery the door decided, where a
 *   copy would settle as the script's `script_error`. It fixes the name and the first line and
 *   adds `timeout`, which the bootstrap says only by its class.
 * - the prelude refused a call before any op: `clientError`, `plainError` and
 *   `unsupportedError`, with the fields the dispatcher would have set for the same refusal
 *   (`not_sent`, never retryable).
 *   `waitTimeoutError` is the same idea for a wait deadline the prelude keeps across ops.
 *
 * The first line names the method the script called (`page.click: ...`), as playwright-core's
 * client prefixes every API error. expect's errors are the exception: Playwright prints those
 * as `expect(locator).toHaveText(expected) failed`, named `Error`, never `TimeoutError`.
 */
import type { OpName } from './ops.generated'

/** What claw-sdk's error envelope says (runtime/bridge.rs `ErrBody`), on every error. */
type ErrorFields = {
  code?: string
  owner?: 'neo' | 'script' | 'website' | 'host'
  delivery?: 'not_sent' | 'done' | 'unknown'
  retryable?: boolean
  operationId?: string
  timeout?: boolean
}

const FIELDS = [
  'code',
  'owner',
  'delivery',
  'retryable',
  'operationId',
  'timeout',
] as const

/** playwright-core's base error. Its `name` stays `Error`, as upstream's does. */
class PlaywrightError extends Error {}

/** Thrown when a Playwright timeout passed; scripts test `error.name === 'TimeoutError'`. */
class TimeoutError extends PlaywrightError {
  constructor(message: string) {
    super(message)
    this.name = 'TimeoutError'
  }
}

function withFields<E extends Error>(error: E, fields: ErrorFields): E {
  for (const field of FIELDS)
    if (fields[field] !== undefined) Reflect.set(error, field, fields[field])
  return error
}

function readFields(thrown: unknown): ErrorFields {
  const fields: ErrorFields = {}
  if (typeof thrown !== 'object' || thrown === null) return fields
  for (const field of FIELDS) {
    const value: unknown = Reflect.get(thrown, field)
    if (value !== undefined) Reflect.set(fields, field, value)
  }
  return fields
}

/**
 * The message with the script's method first. Rust already starts its messages with the
 * header's `api`; the host's own refusals start with the op name (`locator.click: ...`), which
 * becomes the method the script called (`page.click: ...`).
 */
function apiMessage(api: string, op: string, message: string): string {
  if (message.startsWith(`${api}: `)) return message
  if (op !== api && message.startsWith(`${op}: `))
    return `${api}: ${message.slice(op.length + 2)}`
  return `${api}: ${message}`
}

/**
 * Shapes what `__neo.invoke` rejected with into the error the script receives: the
 * bootstrap's own error in place, never a copy (see above). QuickJS captures `stack` as
 * frames only, with no name or message line, so the renamed error's stack stays true.
 */
export function shapeBridgeError(
  op: OpName,
  api: string,
  thrown: unknown,
): Error {
  const read = readFields(thrown)
  const timedOut =
    read.timeout === true ||
    (thrown instanceof Error && thrown.name === 'TimeoutError')
  const assertion = op === 'expect.locator' || op === 'expect.page'
  const named = timedOut && !assertion ? 'TimeoutError' : 'Error'
  if (thrown instanceof Error) {
    if (!assertion) thrown.message = apiMessage(api, op, thrown.message)
    thrown.name = named
    return withFields(thrown, { timeout: timedOut })
  }
  const message =
    typeof thrown === 'object' && thrown !== null
      ? String(Reflect.get(thrown, 'message') ?? thrown)
      : String(thrown)
  const shaped = assertion ? message : apiMessage(api, op, message)
  return withFields(
    named === 'TimeoutError'
      ? new TimeoutError(shaped)
      : new PlaywrightError(shaped),
    { ...read, timeout: timedOut },
  )
}

/** A call the prelude refuses for its arguments: `script / invalid_input`, nothing sent. */
export function clientError(api: string, message: string): Error {
  return withFields(new PlaywrightError(`${api}: ${message}`), {
    code: 'invalid_input',
    owner: 'script',
    delivery: 'not_sent',
    retryable: false,
    timeout: false,
  })
}

/**
 * What upstream's client throws before it calls the server, with no `<api>:` prefix because the
 * throw happens outside the channel call that adds one: "Too many arguments...", "Attempting to
 * serialize unexpected value...", "Unknown polling option...". Fields as `clientError`'s.
 */
export function plainError(message: string): Error {
  return withFields(new PlaywrightError(message), {
    code: 'invalid_input',
    owner: 'script',
    delivery: 'not_sent',
    retryable: false,
    timeout: false,
  })
}

/**
 * A wait whose deadline the prelude keeps across several ops (`waitForEvent` and its re-waits)
 * passed between them: the same `website / wait_timeout` Rust reports when one op times out.
 */
export function waitTimeoutError(api: string, message: string): Error {
  return withFields(new TimeoutError(`${api}: ${message}`), {
    code: 'wait_timeout',
    owner: 'website',
    delivery: 'not_sent',
    retryable: true,
    timeout: true,
  })
}

/** A Playwright method or option Neo lacks: `neo / unsupported`, with what to use instead. */
export function unsupportedError(api: string, hint: string): Error {
  return withFields(new PlaywrightError(`${api}: ${hint}`), {
    code: 'unsupported',
    owner: 'neo',
    delivery: 'not_sent',
    retryable: false,
    timeout: false,
  })
}

/**
 * A matcher that failed in the prelude (value matchers, `toPass`, `expect.poll`): a plain
 * Error as Playwright's expect throws. It carries no claw-sdk fields: no op ran, and Rust's
 * `assertion_failed` means a page assertion, which this is not, so the host classifies it as
 * the script's own error.
 */
export function assertionError(message: string): Error {
  return new PlaywrightError(message)
}

/** Rethrows `error` with the script's method first, for errors the prelude passes through. */
export function prefixed(api: string, error: unknown): unknown {
  if (error instanceof Error && !error.message.startsWith(`${api}: `))
    error.message = `${api}: ${error.message}`
  return error
}
