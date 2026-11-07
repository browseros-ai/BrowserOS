/**
 * Playwright: `client/coverage.ts` `Coverage`
 *
 * `page.coverage`: precise JS coverage and CSS rule usage on the page's own session, from each
 * start to its stop (neo-runtime `coverage.rs`, Playwright's server `CRCoverage`). Every call
 * goes out as Playwright's does, without a timeout.
 */

import type {
  PageStartCSSCoverageParams,
  PageStartJSCoverageParams,
  PageStopCSSCoverageResult,
  PageStopJSCoverageResult,
} from '@browseros/neo-runtime-spec/serve'
import type { Connection } from './connection'
import {
  PAGE_START_CSS_COVERAGE_KEYS,
  PAGE_START_JS_COVERAGE_KEYS,
  pickParams,
} from './protocol'

export class Coverage {
  readonly #connection: Connection
  readonly #guid: string

  constructor(connection: Connection, guid: string) {
    this.#connection = connection
    this.#guid = guid
  }

  async startJSCoverage(
    options: PageStartJSCoverageParams = {},
  ): Promise<void> {
    await this.#connection.sendPage(
      'coverage.startJSCoverage',
      this.#guid,
      'startJSCoverage',
      pickParams(
        PAGE_START_JS_COVERAGE_KEYS,
        options,
      ) as PageStartJSCoverageParams,
      0,
    )
  }

  async stopJSCoverage(): Promise<PageStopJSCoverageResult['entries']> {
    const { entries } = await this.#connection.sendPage(
      'coverage.stopJSCoverage',
      this.#guid,
      'stopJSCoverage',
      {},
      0,
    )
    return entries
  }

  async startCSSCoverage(
    options: PageStartCSSCoverageParams = {},
  ): Promise<void> {
    await this.#connection.sendPage(
      'coverage.startCSSCoverage',
      this.#guid,
      'startCSSCoverage',
      pickParams(
        PAGE_START_CSS_COVERAGE_KEYS,
        options,
      ) as PageStartCSSCoverageParams,
      0,
    )
  }

  async stopCSSCoverage(): Promise<PageStopCSSCoverageResult['entries']> {
    const { entries } = await this.#connection.sendPage(
      'coverage.stopCSSCoverage',
      this.#guid,
      'stopCSSCoverage',
      {},
      0,
    )
    return entries
  }
}
