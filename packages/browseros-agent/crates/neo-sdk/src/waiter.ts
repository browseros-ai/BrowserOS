/**
 * Playwright: `client/waiter.ts` `Waiter`
 * Playwright: `utils/stackTrace.ts` `rewriteErrorMessage`
 *
 * Playwright's client-side wait for an event (`waitForEvent`), run in the realm over the
 * events `events.ts` delivers: it races the event (and its predicate) against rejections on
 * other events and a timeout, and appends its log lines to the error it throws.
 *
 * Left out, as internal to Playwright's tracing: the `__waitInfo__` messages `_sendWaitInfo`
 * sends its server (Neo records no wait steps), and the zone a predicate runs in.
 */

import type { ApiName } from './connection'
import { TimeoutError } from './errors'
import type { EventEmitter } from './events'

/** A `waitForEvent` predicate: may be async, as in Playwright. */
// biome-ignore lint/suspicious/noExplicitAny: the event's value is the listener's to type.
export type EventPredicate = (arg: any) => boolean | Promise<boolean>

/** Playwright's `WaitForEventOptions`: a predicate, or options with one. */
export type WaitForEventOptions =
  | EventPredicate
  | { timeout?: number; predicate?: EventPredicate; signal?: unknown }

/**
 * Deviation: `client/waiter.ts` `Waiter.rejectOnTimeout`: `if (signal) {`
 * QuickJS has no `AbortSignal`, so Neo takes no `signal`; one passed anyway is refused instead
 * of being ignored.
 */
export function refuseSignal(options: { signal?: unknown }): void {
  if (options.signal !== undefined)
    throw new Error('Neo does not support the "signal" option yet')
}

/**
 * Playwright's `ChannelOwner._wrapApiCall` error rewrite for a method the realm runs itself:
 * the `<apiName>: ` prefix on the message. Neo leaves the stack as QuickJS made it.
 */
export async function wrapApiCall<T>(
  apiName: ApiName,
  body: () => Promise<T>,
): Promise<T> {
  try {
    return await body()
  } catch (e) {
    if (e instanceof Error) e.message = `${apiName}: ${e.message}`
    throw e
  }
}

export class Waiter {
  private _dispose: (() => void)[] = []
  private _failures: Promise<unknown>[] = []
  private _immediateError?: Error
  private _logs: string[] = []

  /** Playwright's `createForEvent`; Neo needs neither the owner nor the event for tracing. */
  static createForEvent(): Waiter {
    return new Waiter()
  }

  async waitForEvent<T = void>(
    emitter: EventEmitter,
    event: string,
    predicate?: (arg: T) => boolean | Promise<boolean>,
  ): Promise<T> {
    const { promise, dispose } = waitForEvent(emitter, event, predicate)
    return await this.waitForPromise(promise, dispose)
  }

  rejectOnEvent<T = void>(
    emitter: EventEmitter,
    event: string,
    error: Error | (() => Error),
    predicate?: (arg: T) => boolean | Promise<boolean>,
  ) {
    const { promise, dispose } = waitForEvent(emitter, event, predicate)
    this._rejectOn(
      promise.then(() => {
        throw typeof error === 'function' ? error() : error
      }),
      dispose,
    )
  }

  /** Playwright's `rejectOnTimeout` without its `signal` branch: callers refuse a `signal`. */
  rejectOnTimeout(timeout: number, timeoutMessage: string) {
    if (timeout) {
      const { promise, dispose } = waitForTimeout(timeout)
      this._rejectOn(
        promise.then(() => {
          throw new TimeoutError(timeoutMessage)
        }),
        dispose,
      )
    }
  }

  rejectImmediately(error: Error) {
    this._immediateError = error
  }

  throwIfImmediatelyRejected() {
    if (this._immediateError) throw this._immediateError
  }

  dispose() {
    for (const dispose of this._dispose) dispose()
  }

  async waitForPromise<T>(
    promise: Promise<T>,
    dispose?: () => void,
  ): Promise<T> {
    try {
      if (this._immediateError) throw this._immediateError
      const result = await Promise.race([promise, ...this._failures])
      if (dispose) dispose()
      return result as T
    } catch (e) {
      if (dispose) dispose()
      this.dispose()
      if (e instanceof Error)
        rewriteErrorMessage(e, e.message + formatLogRecording(this._logs))
      throw e
    }
  }

  log(s: string) {
    this._logs.push(s)
  }

  private _rejectOn(promise: Promise<unknown>, dispose?: () => void) {
    this._failures.push(promise)
    if (dispose) this._dispose.push(dispose)
  }
}

/** Playwright: `client/waiter.ts` `waitForEvent` */
function waitForEvent<T = void>(
  emitter: EventEmitter,
  event: string,
  predicate?: (arg: T) => boolean | Promise<boolean>,
): { promise: Promise<T>; dispose: () => void } {
  let listener: (eventArg: T) => void = () => {}
  const promise = new Promise<T>((resolve, reject) => {
    listener = async (eventArg: T) => {
      try {
        if (predicate && !(await predicate(eventArg))) return
        emitter.removeListener(event, listener)
        resolve(eventArg)
      } catch (e) {
        emitter.removeListener(event, listener)
        reject(e)
      }
    }
    emitter.addListener(event, listener)
  })
  const dispose = () => emitter.removeListener(event, listener)
  return { promise, dispose }
}

/** Playwright: `client/waiter.ts` `waitForTimeout` */
function waitForTimeout(timeout: number): {
  promise: Promise<void>
  dispose: () => void
} {
  let timeoutId: ReturnType<typeof setTimeout> | undefined
  const promise = new Promise<void>((resolve) => {
    timeoutId = setTimeout(resolve, timeout)
  })
  const dispose = () => clearTimeout(timeoutId)
  return { promise, dispose }
}

/** Playwright: `client/waiter.ts` `formatLogRecording` */
function formatLogRecording(log: string[]): string {
  if (!log.length) return ''
  const header = ' logs '
  const headerLength = 60
  const leftLength = (headerLength - header.length) / 2
  const rightLength = headerLength - header.length - leftLength
  return `\n${'='.repeat(leftLength)}${header}${'='.repeat(rightLength)}\n${log.join('\n')}\n${'='.repeat(headerLength)}`
}

/** Playwright: `utils/stackTrace.ts` `rewriteErrorMessage` */
export function rewriteErrorMessage<E extends Error>(
  e: E,
  newMessage: string,
): E {
  const lines: string[] = (e.stack?.split('\n') || []).filter((l) =>
    l.startsWith('    at '),
  )
  e.message = newMessage
  const errorTitle = `${e.name}: ${e.message}`
  if (lines.length) e.stack = `${errorTitle}\n${lines.join('\n')}`
  return e
}
