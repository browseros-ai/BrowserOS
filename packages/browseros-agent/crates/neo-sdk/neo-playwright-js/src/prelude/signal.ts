/**
 * Playwright's `signal` option: an `AbortSignal` that aborts one call, as upstream's client
 * aborts a channel call (connection.ts `sendMessageToServer`).
 *
 *   signal already aborted ──▶ AbortError `<api>: The operation was aborted`, nothing sent
 *   signal fires mid-call   ──▶ bootstrap `$neo.abort` ──▶ the op ends as its timeout would,
 *                               code `aborted` ──▶ AbortError `<api>: <reason>` + call log
 *   either way              ──▶ `error.cause` is `signal.reason`, the very value
 *
 * The signal is read by shape, never by class: the realm may have no `AbortSignal` global, and
 * a script's signal may come from another realm's (Node's, in the spec harness).
 */
import { clientError } from './errors'

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

/** `options.signal` when the call has one; anything that is not a signal is refused. */
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
