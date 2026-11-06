/**
 * Playwright's timeout resolution (`client/timeoutSettings.ts` at 26137face), cut to what a run
 * uses: the call's `timeout` option when it is a number (`0` means none), else this object's
 * default, else its parent's, else Playwright's library default. Navigations look up the
 * navigation default first at each level, then the action default. Neo has no inspector, so
 * Playwright's debug-mode branch (`0` under the inspector) is gone.
 */

/** `@isomorphic/time`'s `DEFAULT_PLAYWRIGHT_TIMEOUT`. Playwright Test's default of `0` does not
 * apply: a Neo run is a library call. */
const DEFAULT_PLAYWRIGHT_TIMEOUT = 30_000

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

  /** The timeout in ms an action or read with these options runs under. */
  timeout(options: { timeout?: number }): number {
    if (typeof options.timeout === 'number') return options.timeout
    if (this.#defaultTimeout !== undefined) return this.#defaultTimeout
    if (this.#parent) return this.#parent.timeout(options)
    return DEFAULT_PLAYWRIGHT_TIMEOUT
  }

  /** The timeout in ms a navigation (`goto`) with these options runs under. */
  navigationTimeout(options: { timeout?: number }): number {
    if (typeof options.timeout === 'number') return options.timeout
    if (this.#defaultNavigationTimeout !== undefined)
      return this.#defaultNavigationTimeout
    if (this.#defaultTimeout !== undefined) return this.#defaultTimeout
    if (this.#parent) return this.#parent.navigationTimeout(options)
    return DEFAULT_PLAYWRIGHT_TIMEOUT
  }
}
