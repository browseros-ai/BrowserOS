/**
 * Playwright's timeout resolution (`client/timeoutSettings.ts` at 26137face), cut to what a run
 * uses: the call's `timeout` option when it is a number (`0` means none), else this object's
 * default, else its parent's, else Playwright's library default. Navigations look up the
 * navigation default first at each level, then the action default. Neo has no inspector, so
 * Playwright's debug-mode branch (`0` under the inspector) is gone.
 *
 * `timeout` and `navigationTimeout` return Playwright's `{ signal, timeout }`, the options every
 * call sends with (`connection.ts`): the call rejects with `AbortError` before it is sent when
 * its `signal` already aborted, and stops where it is when the signal aborts later. A method
 * Playwright sends without them (`kNoTimeout`: `isVisible`, `title`, evaluations) sends `0` and
 * reads no signal, as Playwright's does.
 */

import type { ApiName } from './connection'

/** `@isomorphic/time`'s `DEFAULT_PLAYWRIGHT_TIMEOUT`. Playwright Test's default of `0` does not
 * apply: a Neo run is a library call. */
const DEFAULT_PLAYWRIGHT_TIMEOUT = 30_000

/**
 * What Neo reads of a `signal` option: an `AbortSignal` (the realm's, `neo-runtime`
 * `abort.rs`), or any object with its shape, as Playwright's client reads one. `null` is no
 * signal, as `undefined` is.
 */
export type SignalLike = {
  readonly aborted: boolean
  readonly reason?: unknown
  addEventListener(
    type: 'abort',
    listener: () => void,
    options?: { once?: boolean },
  ): void
  removeEventListener(type: 'abort', listener: () => void): void
}

/** The options a call resolves its timeout from: Playwright's `TimeoutInput`. */
export type TimeoutInput = { timeout?: number; signal?: SignalLike | null }

/** Playwright's `channels.TimeoutOptions`: what a call is sent with. */
export type TimeoutOptions = {
  signal: SignalLike | null | undefined
  timeout: number
}

export class TimeoutSettings {
  readonly #parent: TimeoutSettings | undefined
  #defaultTimeout: number | undefined
  #defaultNavigationTimeout: number | undefined

  constructor(parent?: TimeoutSettings) {
    this.#parent = parent
  }

  setDefaultTimeout(timeout: number | undefined): void {
    this.#defaultTimeout = timeout
  }

  setDefaultNavigationTimeout(timeout: number | undefined): void {
    this.#defaultNavigationTimeout = timeout
  }

  /**
   * Playwright: `client/timeoutSettings.ts` `TimeoutSettings.timeout`: `return { signal: options.signal, timeout: this._timeout(options) };`
   *
   * The timeout (ms) and signal an action or read with these options runs under. `apiName`
   * names the calling method, as every caller passes it; resolving never fails, so it is unused.
   */
  timeout(options: TimeoutInput, _apiName: ApiName | null): TimeoutOptions {
    return { signal: options.signal, timeout: this.#timeout(options) }
  }

  /**
   * Playwright: `client/timeoutSettings.ts` `TimeoutSettings.navigationTimeout`: `return { signal: options.signal, timeout: this._navigationTimeout(options) };`
   *
   * The timeout (ms) and signal a navigation (`goto`, `waitForURL`, `waitForLoadState`) with
   * these options runs under. `apiName` as for {@link TimeoutSettings.timeout}.
   */
  navigationTimeout(
    options: TimeoutInput,
    _apiName: ApiName | null,
  ): TimeoutOptions {
    return { signal: options.signal, timeout: this.#navigationTimeout(options) }
  }

  #timeout(options: TimeoutInput): number {
    if (typeof options.timeout === 'number') return options.timeout
    if (this.#defaultTimeout !== undefined) return this.#defaultTimeout
    if (this.#parent) return this.#parent.#timeout(options)
    return DEFAULT_PLAYWRIGHT_TIMEOUT
  }

  #navigationTimeout(options: TimeoutInput): number {
    if (typeof options.timeout === 'number') return options.timeout
    if (this.#defaultNavigationTimeout !== undefined)
      return this.#defaultNavigationTimeout
    if (this.#defaultTimeout !== undefined) return this.#defaultTimeout
    if (this.#parent) return this.#parent.#navigationTimeout(options)
    return DEFAULT_PLAYWRIGHT_TIMEOUT
  }
}
