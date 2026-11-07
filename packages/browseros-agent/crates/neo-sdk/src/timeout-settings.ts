/**
 * Playwright's timeout resolution (`client/timeoutSettings.ts` at 26137face), cut to what a run
 * uses: the call's `timeout` option when it is a number (`0` means none), else this object's
 * default, else its parent's, else Playwright's library default. Navigations look up the
 * navigation default first at each level, then the action default. Neo has no inspector, so
 * Playwright's debug-mode branch (`0` under the inspector) is gone.
 *
 * Playwright's `timeout` and `navigationTimeout` return `{ signal, timeout }`, and the client
 * aborts the call when its `signal` aborts. Neo cannot abort a call it has sent, so both refuse
 * any `signal` instead, before the call is sent: every method that resolves its timeout here
 * (each Frame method, which every Locator and Page method reaches) rejects with an `Error`
 * naming the option, and the browser sees nothing. Silently dropping the signal would let a
 * script believe it could cancel the call.
 */

import type { ApiName } from './connection'
import { refuseSignal } from './waiter'

/** `@isomorphic/time`'s `DEFAULT_PLAYWRIGHT_TIMEOUT`. Playwright Test's default of `0` does not
 * apply: a Neo run is a library call. */
const DEFAULT_PLAYWRIGHT_TIMEOUT = 30_000

/** The options a call resolves its timeout from: Playwright's `TimeoutInput`. */
export type TimeoutInput = { timeout?: number; signal?: unknown }

/**
 * Deviation: `client/timeoutSettings.ts` `TimeoutSettings.timeout`: `return { signal: options.signal, timeout: this._timeout(options) };` Playwright hands the signal to the call, which aborts when it aborts; Neo has no way to abort a sent call yet, so it refuses any signal, an aborted one included, before sending anything; null and undefined are none.
 *
 * `waiter.ts`'s `refuseSignal`, the one refusal, prefixed with `apiName` as Playwright's
 * `_wrapApiCall` prefixes a client-side error; `null` inside a `wrapApiCall` body, which adds
 * the prefix itself.
 */
function refuse(options: TimeoutInput, apiName: ApiName | null): void {
  try {
    refuseSignal(options)
  } catch (error) {
    if (apiName !== null && error instanceof Error)
      error.message = `${apiName}: ${error.message}`
    throw error
  }
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
   * The timeout in ms an action or read with these options runs under.
   *
   * @throws `Error` prefixed with `apiName` (unless `null`) when the options carry a `signal`.
   */
  timeout(options: TimeoutInput, apiName: ApiName | null): number {
    refuse(options, apiName)
    return this.#timeout(options)
  }

  /**
   * The timeout in ms a navigation (`goto`, `waitForURL`, `waitForLoadState`) with these
   * options runs under.
   *
   * @throws `Error` prefixed with `apiName` (unless `null`) when the options carry a `signal`.
   */
  navigationTimeout(options: TimeoutInput, apiName: ApiName | null): number {
    refuse(options, apiName)
    return this.#navigationTimeout(options)
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
