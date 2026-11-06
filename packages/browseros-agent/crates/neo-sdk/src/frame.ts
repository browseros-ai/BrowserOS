/**
 * Playwright's `Frame` (`client/frame.ts` at 26137face), cut to a page's main frame: its guid,
 * its page, the selector builders, whose bodies are Playwright's, and `click`. A page builds
 * exactly one `Frame` per run, because `Locator` compares frames by identity.
 *
 * `_click` is where every public click (`locator.click`, `page.click`, `frame.click`) becomes a
 * protocol message, with its caller's `apiName`.
 */

import type { FrameClickParams } from '@browseros/neo-runtime-spec/serve'
import type { ByRoleOptions } from '../vendor/playwright/isomorphic/locatorUtils'
import {
  getByAltTextSelector,
  getByLabelSelector,
  getByPlaceholderSelector,
  getByRoleSelector,
  getByTestIdSelector,
  getByTextSelector,
  getByTitleSelector,
} from '../vendor/playwright/isomorphic/locatorUtils'
import type { ApiName, Connection } from './connection'
import { Locator, type LocatorOptions, testIdAttributeName } from './locator'
import type { Page } from './page'
import { FRAME_CLICK_KEYS, pickParams } from './protocol'

/** `click`'s options as a script passes them: the wire params plus `timeout`. */
export type ClickOptions = Omit<FrameClickParams, 'selector'> & {
  timeout?: number
}

export class Frame {
  /** `frame@<targetId>`, the guid neo-runtime routes this frame's browser calls by. */
  readonly _guid: string
  readonly _page: Page
  readonly _connection: Connection

  constructor(guid: string, page: Page, connection: Connection) {
    this._guid = guid
    this._page = page
    this._connection = connection
  }

  page(): Page {
    return this._page
  }

  /** Not strict, as Playwright's `frame.click`. */
  async click(selector: string, options: ClickOptions = {}): Promise<void> {
    return await this._click('frame.click', selector, options)
  }

  /** Sends `Frame.click`. Runs synchronously into `Connection.send`, so the location it
   * captures is the script's call site. */
  async _click(
    apiName: ApiName,
    selector: string,
    options: ClickOptions,
  ): Promise<void> {
    await this._connection.send(
      apiName,
      this._guid,
      'click',
      pickParams(FRAME_CLICK_KEYS, {
        selector,
        ...options,
      }) as FrameClickParams,
      this._page._timeoutSettings.timeout(options),
    )
  }

  locator(selector: string, options?: LocatorOptions): Locator {
    return new Locator(this, selector, options)
  }

  getByTestId(testId: string | RegExp): Locator {
    return this.locator(getByTestIdSelector(testIdAttributeName(), testId))
  }

  getByAltText(text: string | RegExp, options?: { exact?: boolean }): Locator {
    return this.locator(getByAltTextSelector(text, options))
  }

  getByLabel(text: string | RegExp, options?: { exact?: boolean }): Locator {
    return this.locator(getByLabelSelector(text, options))
  }

  getByPlaceholder(
    text: string | RegExp,
    options?: { exact?: boolean },
  ): Locator {
    return this.locator(getByPlaceholderSelector(text, options))
  }

  getByText(text: string | RegExp, options?: { exact?: boolean }): Locator {
    return this.locator(getByTextSelector(text, options))
  }

  getByTitle(text: string | RegExp, options?: { exact?: boolean }): Locator {
    return this.locator(getByTitleSelector(text, options))
  }

  getByRole(role: string, options: ByRoleOptions = {}): Locator {
    return this.locator(getByRoleSelector(role, options))
  }
}
