/**
 * One `waitForEvent`'s deadline: the call's `since` stamp plus its timeout, or none for 0.
 *
 * Rust times each `events.wait` from the op's own start, so the prelude keeps the deadline that
 * spans a wait and its re-waits: each op gets the time left, and a pending predicate races the
 * rest, as upstream's Waiter races its timeout against the event. However the deadline passes,
 * the script gets one TimeoutError that names the timeout it asked for.
 */
import { startTimer } from './bridge'
import { waitTimeoutError } from './errors'

export class EventDeadline {
  private readonly api: string
  private readonly event: string
  private readonly timeout: number
  /** The `Date.now()` the wait times out at; null for a timeout of 0, which means none. */
  private readonly endsAt: number | null

  constructor(api: string, event: string, since: number, timeout: number) {
    this.api = api
    this.event = event
    this.timeout = timeout
    this.endsAt = timeout === 0 ? null : since + timeout
  }

  /**
   * The next op's `timeoutMs`: the time left, or 0 for no deadline. Throws once none is left,
   * because a 0 would tell Rust to wait forever.
   */
  left(): number {
    if (this.endsAt === null) return 0
    const left = this.endsAt - Date.now()
    if (left <= 0) throw this.expired()
    return left
  }

  /** An op's failure; a timeout names the script's timeout, not the time left that Rust saw. */
  named(error: unknown): unknown {
    if (
      !(error instanceof Error) ||
      Reflect.get(error, 'code') !== 'wait_timeout'
    )
      return error
    const [, ...log] = error.message.split('\n')
    error.message = [`${this.api}: ${this.message()}`, ...log].join('\n')
    return error
  }

  /**
   * The predicate's verdict, or the TimeoutError if the deadline passes while it is pending. A
   * verdict that is not a promise is final at once, as it is upstream.
   */
  race(verdict: unknown): Promise<unknown> {
    const endsAt = this.endsAt
    if (endsAt === null || !isThenable(verdict)) return Promise.resolve(verdict)
    let cancel = () => {}
    const expired = new Promise<never>((_, reject) => {
      cancel = startTimer(Math.max(0, endsAt - Date.now()), () =>
        reject(this.expired()),
      )
    })
    return Promise.race([verdict, expired]).finally(cancel)
  }

  private expired(): Error {
    return waitTimeoutError(this.api, this.message())
  }

  private message(): string {
    return `Timeout ${this.timeout}ms exceeded while waiting for event "${this.event}"`
  }
}

function isThenable(value: unknown): value is PromiseLike<unknown> {
  return (
    (typeof value === 'object' || typeof value === 'function') &&
    value !== null &&
    typeof Reflect.get(value, 'then') === 'function'
  )
}
