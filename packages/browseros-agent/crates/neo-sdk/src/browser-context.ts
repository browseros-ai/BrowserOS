import { TimeoutSettings } from './timeout-settings'

/** Playwright's `BrowserContext`. Scripts receive it as `context`. It serves no browser call
 * yet, and holds the default timeout its pages fall back to. */
export class BrowserContext {
  readonly _timeoutSettings = new TimeoutSettings()

  setDefaultTimeout(timeout: number | undefined): void {
    this._timeoutSettings.setDefaultTimeout(timeout)
  }
}
