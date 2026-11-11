/**
 * Playwright's `signal` option: an `AbortSignal` that aborts one call, as upstream's client
 * aborts a channel call (connection.ts `sendMessageToServer`).
 *
 *   aborted when the call starts ──▶ AbortError `<api>: The operation was aborted`, nothing
 *                                    sent (`signalOf`, read before the call's first await)
 *   fires once the call started  ──▶ the op is sent, the bootstrap's `$neo.abort` follows it ──▶
 *                                    the op ends as its timeout would, code `aborted` ──▶
 *                                    AbortError `<api>: <reason>` + call log
 *   a wait upstream's Waiter runs ──▶ the whole wait rejects with `<api>: The operation was
 *   (waitForEvent, waitForURL,       aborted`, and the op in flight is aborted in Rust too
 *    waitForLoadState)              (`untilAborted`)
 *   every way                    ──▶ `error.cause` is `signal.reason`, the very value
 *
 * Upstream's client reaches its channel send with no await, so a signal that fires after the
 * call returned its promise always finds the call in flight. Neo's prelude awaits before it
 * sends (a page brand, a frame's arguments), so the "already aborted" check happens at the
 * call itself, and a signal that fires during those awaits still sends the op, which the
 * bootstrap aborts at once.
 *
 * The signal is read by shape, never by class: the realm may have no `AbortSignal` global, and
 * a script's signal may come from another realm's (Node's, in the spec harness).
 */
import { abortedError, clientError } from './errors'

/** What Neo reads from an `AbortSignal`. */
export type SignalLike = {
  readonly aborted: boolean
  readonly reason: unknown
  addEventListener(
    type: 'abort',
    listener: () => void,
    options?: { once?: boolean },
  ): void
  removeEventListener(type: 'abort', listener: () => void): void
}

/** The options of a call that takes `signal`. */
export type SignalOption = { signal?: SignalLike }

/**
 * `options.signal` when the call has one; anything that is not a signal is refused, and a
 * signal already aborted throws the call's AbortError. Call it before the method's first await.
 */
export function signalOf(
  api: string,
  options: unknown,
): SignalLike | undefined {
  if (typeof options !== 'object' || options === null) return undefined
  const signal: unknown = Reflect.get(options, 'signal')
  if (signal === undefined) return undefined
  if (
    typeof signal !== 'object' ||
    signal === null ||
    typeof Reflect.get(signal, 'aborted') !== 'boolean' ||
    typeof Reflect.get(signal, 'addEventListener') !== 'function' ||
    typeof Reflect.get(signal, 'removeEventListener') !== 'function'
  )
    throw clientError(api, 'signal: expected an AbortSignal')
  if ((signal as SignalLike).aborted)
    throw abortedError(api, signal as SignalLike)
  return signal as SignalLike
}

/**
 * Runs `work` until it settles or `signal` fires, whichever is first; `aborted` builds the error
 * an abort rejects with. For a wait the prelude keeps across several ops (`waitForEvent`), where
 * upstream's Waiter rejects the whole wait at once; each op inside it takes the signal too, so
 * the one in flight ends in Rust as well.
 */
export async function untilAborted<T>(
  signal: SignalLike | undefined,
  aborted: () => Error,
  work: () => Promise<T>,
): Promise<T> {
  if (signal === undefined) return await work()
  if (signal.aborted) throw aborted()
  let detach = () => {}
  const abort = new Promise<never>((_, reject) => {
    const listener = () => reject(aborted())
    signal.addEventListener('abort', listener, { once: true })
    detach = () => signal.removeEventListener('abort', listener)
  })
  try {
    return await Promise.race([work(), abort])
  } finally {
    detach()
  }
}
