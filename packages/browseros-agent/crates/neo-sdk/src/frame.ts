/**
 * Playwright's `Frame` (`client/frame.ts` at 26137face), cut to a page's main frame: its guid,
 * its page, the selector builders, whose bodies are Playwright's, `goto`, `title`, `textContent`
 * and `click`. A page builds exactly one `Frame` per run, because `Locator` compares frames by
 * identity.
 *
 * Each `_<method>` is where every public form of a browser call (`locator.textContent`,
 * `page.textContent`, `frame.textContent`) becomes a protocol message, with its caller's
 * `apiName`. Each runs synchronously into `Connection.send`, so the location it captures is the
 * script's call site.
 *
 * What differs from Playwright: `goto` resolves `null` for every navigation, because Neo creates
 * no `Response` objects yet (Playwright returns `null` only for navigations without one, such as
 * `about:blank` or a same-document change).
 */

import type {
  FrameAriaSnapshotParams,
  FrameAriaSnapshotResult,
  FrameClickParams,
  FrameGotoParams,
  FrameTextContentParams,
  FrameTextContentResult,
  FrameTitleResult,
  LifecycleEvent,
} from '@browseros/neo-runtime-spec/serve'
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
import {
  FRAME_ARIA_SNAPSHOT_KEYS,
  FRAME_CLICK_KEYS,
  FRAME_GOTO_KEYS,
  FRAME_TEXT_CONTENT_KEYS,
  pickParams,
} from './protocol'

/** `click`'s options as a script passes them: the wire params plus `timeout`. */
export type ClickOptions = Omit<FrameClickParams, 'selector'> & {
  timeout?: number
}

/** `goto`'s options as a script passes them: the wire params plus `timeout`. */
export type GotoOptions = Omit<FrameGotoParams, 'url'> & { timeout?: number }

/** `textContent`'s options as a script passes them. */
export type TextContentOptions = Omit<FrameTextContentParams, 'selector'> & {
  timeout?: number
}

/** `page.ariaSnapshot`'s options, as Playwright's `Page.ariaSnapshot` takes them. */
export type AriaSnapshotOptions = {
  mode?: 'ai' | 'default'
  depth?: number
  boxes?: boolean
  timeout?: number
}

/** Playwright's `kLifecycleEvents` (`client/types.ts`). */
const LIFECYCLE_EVENTS: ReadonlySet<string> = new Set([
  'load',
  'domcontentloaded',
  'networkidle',
  'commit',
])

/**
 * Playwright's `verifyLoadState` (`client/frame.ts`): `networkidle0` is an alias, and any other
 * unknown value throws a plain `Error` before a message is sent, so it has no `page.goto: `
 * prefix.
 */
export function verifyLoadState(
  name: string,
  waitUntil: LifecycleEvent,
): LifecycleEvent {
  if ((waitUntil as unknown) === 'networkidle0') waitUntil = 'networkidle'
  if (!LIFECYCLE_EVENTS.has(waitUntil))
    throw new Error(
      `${name}: expected one of (load|domcontentloaded|networkidle|commit)`,
    )
  return waitUntil
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

  /** Resolves `null`: Neo creates no `Response` objects yet. */
  async goto(url: string, options: GotoOptions = {}): Promise<null> {
    return await this._goto('frame.goto', url, options)
  }

  /** Sends `Frame.goto` under the navigation timeout, `waitUntil` defaulting to `load`. */
  async _goto(
    apiName: ApiName,
    url: string,
    options: GotoOptions,
  ): Promise<null> {
    const waitUntil = verifyLoadState(
      'waitUntil',
      options.waitUntil === undefined ? 'load' : options.waitUntil,
    )
    await this._connection.send(
      apiName,
      this._guid,
      'goto',
      pickParams(FRAME_GOTO_KEYS, {
        url,
        ...options,
        waitUntil,
      }) as FrameGotoParams,
      this._page._timeoutSettings.navigationTimeout(options),
    )
    return null
  }

  async title(): Promise<string> {
    return await this._title('frame.title')
  }

  /** Sends `Frame.title` with no timeout, as Playwright's `kNoTimeout`. */
  async _title(apiName: ApiName): Promise<string> {
    const result = (await this._connection.send(
      apiName,
      this._guid,
      'title',
      {},
      0,
    )) as FrameTitleResult
    return result.value
  }

  /** Not strict, as Playwright's `frame.textContent`. */
  async textContent(
    selector: string,
    options: TextContentOptions = {},
  ): Promise<string | null> {
    return await this._textContent('frame.textContent', selector, options)
  }

  async _textContent(
    apiName: ApiName,
    selector: string,
    options: TextContentOptions,
  ): Promise<string | null> {
    const result = (await this._connection.send(
      apiName,
      this._guid,
      'textContent',
      pickParams(FRAME_TEXT_CONTENT_KEYS, {
        selector,
        ...options,
      }) as FrameTextContentParams,
      this._page._timeoutSettings.timeout(options),
    )) as FrameTextContentResult
    return result.value === undefined ? null : result.value
  }

  /** Sends `Frame.ariaSnapshot` for the whole frame: no selector, as `page.ariaSnapshot`. */
  async _ariaSnapshot(
    apiName: ApiName,
    options: AriaSnapshotOptions,
  ): Promise<string> {
    const result = (await this._connection.send(
      apiName,
      this._guid,
      'ariaSnapshot',
      pickParams(FRAME_ARIA_SNAPSHOT_KEYS, {
        mode: options.mode,
        depth: options.depth,
        boxes: options.boxes,
      }) as FrameAriaSnapshotParams,
      this._page._timeoutSettings.timeout(options),
    )) as FrameAriaSnapshotResult
    return result.snapshot
  }

  /** Not strict, as Playwright's `frame.click`. */
  async click(selector: string, options: ClickOptions = {}): Promise<void> {
    return await this._click('frame.click', selector, options)
  }

  /** Sends `Frame.click`. */
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
