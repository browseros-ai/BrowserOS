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
 * - the prelude refused a call before any op: `unsupportedError` for a method, option or event
 *   Neo does not serve (`script / method_unavailable`), and `clientError`, `plainError` and
 *   `unprefixedClientError` for its arguments (`script / invalid_input`). Each is a door call
 *   (FR/RC, the bootstrap's `__neo.refuse`): the door records the try as a row of the run's
 *   trace and decides the error's code, owner, delivery and op id, as for any op it refuses,
 *   and the error is the bootstrap's own NeoError, so an uncaught one settles the run with that
 *   code. The door's fields reach the error only after the script next yields to Rust. A
 *   bootstrap without `refuse` gets the error each was before FR/RC.
 *   `waitTimeoutError` is the same idea for a wait deadline the prelude keeps across ops, and
 *   is not a refusal: Rust's own wait would have timed out.
 *
 * An op the script aborted through `signal` (code `aborted`) is named `AbortError`, as upstream's
 * client names one, and gets the signal's reason as its `cause` (signal.ts).
 *
 * The first line names the method the script called (`page.click: ...`), as playwright-core's
 * client prefixes every API error. expect's errors are the exception: Playwright prints those
 * as `expect(locator).toHaveText(expected) failed`, named `Error`, never `TimeoutError`.
 */

import type { ErrBody } from './generated/envelope'
import type { OpName } from './generated/ops'
import { localTimeout, type RefusalCode, refuser } from './host'
import type { SignalLike } from './signal'

/** The fields of claw-sdk's error envelope (runtime/bridge.rs `ErrBody`) every error carries. */
const FIELDS = [
  'code',
  'owner',
  'delivery',
  'retryable',
  'operationId',
  'timeout',
] as const satisfies readonly (keyof ErrBody)[]

/** Those fields as an error has them: each one the envelope or the prelude supplied. */
type ErrorFields = Partial<Pick<ErrBody, (typeof FIELDS)[number]>>

/** playwright-core's base error. Its `name` stays `Error`, as upstream's does. */
class PlaywrightError extends Error {}

/** Thrown when a Playwright timeout passed; scripts test `error.name === 'TimeoutError'`. */
class TimeoutError extends PlaywrightError {
  constructor(message: string) {
    super(message)
    this.name = 'TimeoutError'
  }
}

/** Thrown when the script aborted the call through `signal`, named as Node names one. */
class AbortError extends PlaywrightError {
  constructor(message: string, cause: unknown) {
    super(message, { cause })
    this.name = 'AbortError'
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
  const named = assertion
    ? 'Error'
    : read.code === 'aborted'
      ? 'AbortError'
      : timedOut
        ? 'TimeoutError'
        : 'Error'
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
      : named === 'AbortError'
        ? new AbortError(shaped, undefined)
        : new PlaywrightError(shaped),
    { ...read, timeout: timedOut },
  )
}

/**
 * Gives an op that `signal` aborted the signal's reason as its `cause`, the very value, as
 * upstream's client does once the server answered the abort (connection.ts `dispatch`). Any
 * other failure passes through: an op can fail for its own reason after the signal fired.
 */
export function withAbortCause(
  error: Error,
  signal: SignalLike | undefined,
): Error {
  if (signal?.aborted && error.name === 'AbortError')
    Object.defineProperty(error, 'cause', {
      value: signal.reason,
      writable: true,
      configurable: true,
    })
  return error
}

/**
 * A call whose `signal` was aborted before it was sent, as upstream's client throws it without a
 * channel call: `<api>: The operation was aborted`, its cause the signal's reason. It carries
 * claw-sdk's fields with nothing sent, but no op: the door never saw it, so an uncaught one ends
 * the run as the script's own error, which it is.
 */
export function abortedError(api: string, signal: SignalLike): Error {
  return withFields(
    new AbortError(`${api}: The operation was aborted`, signal.reason),
    {
      code: 'aborted',
      owner: 'script',
      delivery: 'not_sent',
      retryable: true,
      timeout: false,
    },
  )
}

/** An argument refusal's fields from a bootstrap without `refuse`, as before FR/RC. */
const REFUSED_ARGUMENTS: ErrorFields = {
  code: 'invalid_input',
  owner: 'script',
  delivery: 'not_sent',
  retryable: false,
  timeout: false,
}

/** An unserved method's fields from a bootstrap without `refuse`, as before FR/RC. */
const UNSUPPORTED: ErrorFields = {
  code: 'unsupported',
  owner: 'neo',
  delivery: 'not_sent',
  retryable: false,
  timeout: false,
}

/**
 * A call the prelude refuses before any op, recorded by the door under `api`, the method the
 * script called. `message` is the whole text the script reads, the same with either bootstrap.
 * The error is the one `__neo.refuse` returns, renamed in place to Playwright's `name`: a
 * wrapped or copied error would settle as the script's own and lose the refusal's code.
 * Without `refuse` (an older bootstrap) it is the error the prelude threw before, with
 * `fallback`'s fields.
 */
function refusal(
  api: string,
  code: RefusalCode,
  message: string,
  fallback: ErrorFields,
  name: 'Error' | 'TypeError' = 'Error',
): Error {
  const refuse = refuser()
  if (refuse === null)
    return withFields(
      name === 'TypeError'
        ? new TypeError(message)
        : new PlaywrightError(message),
      fallback,
    )
  const error = refuse(api, code, message)
  error.name = name
  return withFields(error, { timeout: false })
}

/** A call the prelude refuses for its arguments: `script / invalid_input`, nothing sent. */
export function clientError(api: string, message: string): Error {
  return refusal(api, 'invalid_input', `${api}: ${message}`, REFUSED_ARGUMENTS)
}

/**
 * The same refusal where upstream's client throws before any channel call, which gives its
 * error no method prefix (`File paths cannot be mixed with buffers`). `TypeError` is for what
 * V8 throws there on a missing argument (`Cannot read properties of undefined (reading …)`):
 * the refusal carries its name, though it is not an instance of `TypeError`.
 */
export function unprefixedClientError(
  api: string,
  message: string,
  kind: 'Error' | 'TypeError' = 'Error',
): Error {
  return refusal(api, 'invalid_input', message, REFUSED_ARGUMENTS, kind)
}

/**
 * What upstream's client throws before it calls the server, with no `<api>:` prefix because the
 * throw happens outside the channel call that adds one: "Too many arguments...", "Attempting to
 * serialize unexpected value...", "Unknown polling option...". Recorded under `api` as
 * `clientError`'s refusals are.
 */
export function plainError(api: string, message: string): Error {
  return refusal(api, 'invalid_input', message, REFUSED_ARGUMENTS)
}

/**
 * A Neo-side inconsistency the prelude noticed (`newPage` answered with no page): a plain error,
 * not a refusal, since only Rust can vouch for a failure Neo owns.
 */
export function internalError(api: string, message: string): Error {
  return new PlaywrightError(`${api}: ${message}`)
}

/**
 * A wait whose deadline the prelude keeps across several ops (`waitForEvent` and its re-waits)
 * passed between them: the same `website / wait_timeout` Rust reports when one op times out.
 */
export function waitTimeoutError(api: string, message: string): Error {
  const text = `${api}: ${message}`
  return withFields(localTimeout(text) ?? new TimeoutError(text), {
    code: 'wait_timeout',
    owner: 'website',
    delivery: 'not_sent',
    retryable: true,
    timeout: true,
  })
}

/**
 * A Playwright method, option or event Neo lacks, with what to use instead:
 * `script / method_unavailable`, the door's code for a method it does not serve.
 */
export function unsupportedError(api: string, hint: string): Error {
  return refusal(api, 'method_unavailable', `${api}: ${hint}`, UNSUPPORTED)
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

/**
 * An op's answer the prelude could not read, as when QuickJS's stack overflows in a deep
 * result's parse: the op's failure, `<api>: <message>`, named `Error` as the bootstrap's
 * rejection of an op is once shaped. It carries no claw-sdk fields, since the op itself
 * answered, so an uncaught one ends the run as the script's own error.
 */
export function answerError(api: string, thrown: unknown): Error {
  const message = thrown instanceof Error ? thrown.message : String(thrown)
  return new PlaywrightError(`${api}: ${message}`)
}

/** Rethrows `error` with the script's method first, for errors the prelude passes through. */
export function prefixed(api: string, error: unknown): unknown {
  if (error instanceof Error && !error.message.startsWith(`${api}: `))
    error.message = `${api}: ${error.message}`
  return error
}
