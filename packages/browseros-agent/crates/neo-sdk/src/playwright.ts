/**
 * Playwright: `client/playwright.ts` `Playwright`
 *
 * Playwright's `Playwright` object, a run's third parameter (`async function (page, context,
 * playwright)`): the browser types, Playwright's device descriptors, its public error classes,
 * `selectors` and `request`. Neo's run is attached to BrowserOS, so `chromium` launches nothing
 * (`browser-type.ts`); the browser is `context.browser()`.
 */

import { BrowserType } from './browser-type'
import type { Connection } from './connection'
import { TimeoutError } from './errors'
import { setTestIdAttribute } from './locator'
import { devices } from './runtime-bridge'

/** Playwright's `DeviceDescriptor`, a `newContext` options bag. */
export interface DeviceDescriptor {
  userAgent: string
  viewport: { width: number; height: number }
  screen?: { width: number; height: number }
  deviceScaleFactor: number
  isMobile: boolean
  hasTouch: boolean
  defaultBrowserType: 'chromium' | 'firefox' | 'webkit'
}

/**
 * Playwright: `client/selectors.ts` `Selectors`
 *
 * Playwright's `selectors`. `setTestIdAttribute` changes the attribute every later
 * `getByTestId` selects by, in every frame, as Playwright's.
 */
export class Selectors {
  #connection: Connection | undefined
  #context: string | undefined

  /** Neo: where `setTestIdAttribute` sends its page half: the run's connection and context. */
  _attach(connection: Connection, context: string): void {
    this.#connection = connection
    this.#context = context
  }

  /**
   * Deviation: `client/selectors.ts` `Selectors.register`: `await context._channel.registerSelectorEngine({ selectorEngine }, kNoTimeout);`
   * A custom engine is the script's own code run in every page, a new door into page JS that
   * Neo's page JS guard does not open; it refuses before anything reaches a page.
   */
  async register(
    _name: string,
    _script: unknown,
    _options?: { contentScript?: boolean },
  ): Promise<void> {
    throw new Error(
      'selectors.register: Neo does not run custom selector engines in the page',
    )
  }

  /**
   * Playwright's `Selectors.setTestIdAttribute`: the attribute `getByTestId` builds its selector
   * with from now on, in this run, and the one the injected script of every document the run
   * reaches afterwards names in the locators a strict mode violation suggests (a document already
   * open keeps its own, as in Playwright). Rust takes it inside the call's native send, so every
   * later call of the run sees it.
   */
  setTestIdAttribute(attributeName: string): void {
    setTestIdAttribute(attributeName)
    if (!this.#connection || !this.#context) return
    // Playwright: `client/selectors.ts` `Selectors.setTestIdAttribute`: `context._channel.setTestIdAttributeName({ testIdAttributeName: attributeName }, kNoTimeout).catch(() => {});`
    this.#connection
      .sendContext(
        'selectors.setTestIdAttribute',
        this.#context,
        'setTestIdAttributeName',
        { testIdAttributeName: attributeName },
        0,
      )
      .catch(() => {})
  }
}

/**
 * Deviation: `client/playwright.ts` `Playwright.constructor`: `this.request = new APIRequest(this);`
 * Neo does not serve API request contexts yet (the network area's `APIRequestContext`); a script
 * that asks for one gets an Error, never a stand-in.
 */
export class APIRequest {
  async newContext(_options?: unknown): Promise<never> {
    throw new Error(
      'apiRequest.newContext: Neo does not serve playwright.request yet',
    )
  }
}

export class Playwright {
  readonly chromium = new BrowserType('chromium')
  readonly firefox = new BrowserType('firefox')
  readonly webkit = new BrowserType('webkit')
  readonly selectors = new Selectors()
  readonly request = new APIRequest()
  /** Playwright: `client/playwright.ts` `Playwright.constructor`: `this.errors = { TimeoutError };` */
  readonly errors = { TimeoutError }
  #devices: Record<string, DeviceDescriptor> | undefined

  /**
   * Playwright's device descriptors at the pin, by name (`devices['iPhone 13']`), to spread into
   * `browser.newContext`. Read from neo-runtime the first time a script asks, so a run that never
   * does pays nothing for them; the same object every time after.
   */
  get devices(): Record<string, DeviceDescriptor> {
    this.#devices ??= devices() as Record<string, DeviceDescriptor>
    return this.#devices
  }
}
