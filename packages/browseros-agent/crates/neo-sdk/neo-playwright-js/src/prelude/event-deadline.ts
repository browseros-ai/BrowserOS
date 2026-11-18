/**
 * One call's deadline kept across several ops: the call's `since` stamp plus its timeout, or none
 * for 0.
 *
 * Rust times each op from the op's own start, so the prelude keeps the deadline that spans a wait
 * and its re-waits: each op gets the time left, and a pending predicate races the rest, as
 * upstream's Waiter races its timeout against the event. However the deadline passes, the script
 * gets one TimeoutError that names the timeout it asked for.
 *
 * Two rules keep the verdict where it belongs:
 * - A result the call already holds is the call's, whatever the clock says by then. A deadline
 *   check after it would be a second, later verdict on it (a snapshot's tree arrived, then the
 *   call timed out with no op behind it). Work after it that only keeps Neo's own state (a
 *   snapshot's delta baseline, a click's event fence, a navigation's frame refresh) asks
 *   `remaining()` or `within()` and is skipped or cut short when the time is gone, never failed.
 *   A wait still in flight may race the deadline, as upstream's Waiter does: that result is not
 *   held yet.
 * - A timeout the prelude raises itself (`left()` with none left, `race()` running out) has no op
 *   behind it, so an uncaught one ends the run as the script's own error: the realm cannot vouch
 *   for a timeout a script could build the same way. Where the verdict matters, an op carries it
 *   and Rust decides (a snapshot's baseline ops, snapshot-late.ts).
 */
import { startTimer } from './bridge'
import { waitTimeoutError } from './errors'

export class EventDeadline {
  private readonly api: string
  private readonly event: string | null
  private readonly timeout: number
  /** The `Date.now()` the wait times out at; null for a timeout of 0, which means none. */
  private readonly endsAt: number | null

  /** A null event uses waitForNavigation's plain timeout line. */
  constructor(
    api: string,
    event: string | null,
    since: number,
    timeout: number,
  ) {
    this.api = api
    this.event = event
    this.timeout = timeout
    this.endsAt = timeout === 0 ? null : since + timeout
  }

  /**
   * The next op's `timeoutMs`: the time left, or 0 for no deadline. Throws the wait's timeout once
   * none is left, because a 0 would tell Rust to wait forever: only before an op whose result
   * the call still needs, never after one it already has.
   */
  left(): number {
    const left = this.remaining()
    if (left === null) throw this.expired()
    return left
  }

  /**
   * `left()` for work the call can do without: the time left, 0 for no deadline, or null once
   * none is left, for the caller to skip the work rather than fail the call.
   */
  remaining(): number | null {
    if (this.endsAt === null) return 0
    const left = this.endsAt - Date.now()
    return left > 0 ? left : null
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
  race<T>(verdict: T): Promise<Awaited<T>> {
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

  /**
   * Waits for `work` until the deadline at most, for work the call can do without: its value,
   * or null when the deadline passed first, which is no failure. Its rejection still rejects.
   * Work left running past the deadline settles unobserved.
   */
  async within<T>(work: Promise<T>): Promise<{ value: T } | null> {
    const endsAt = this.endsAt
    if (endsAt === null) return { value: await work }
    let cancel = () => {}
    const passed = new Promise<null>((resolve) => {
      cancel = startTimer(Math.max(0, endsAt - Date.now()), () => resolve(null))
    })
    const done = work.then((value) => ({ value }))
    return await Promise.race([done, passed]).finally(cancel)
  }

  private expired(): Error {
    return waitTimeoutError(this.api, this.message())
  }

  private message(): string {
    return this.event === null
      ? `Timeout ${this.timeout}ms exceeded.`
      : `Timeout ${this.timeout}ms exceeded while waiting for event "${this.event}"`
  }
}

function isThenable(value: unknown): value is PromiseLike<unknown> {
  return (
    (typeof value === 'object' || typeof value === 'function') &&
    value !== null &&
    typeof Reflect.get(value, 'then') === 'function'
  )
}
