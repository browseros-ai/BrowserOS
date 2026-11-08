/**
 * Playwright: `client/browserType.ts` `BrowserType`
 *
 * Playwright's `BrowserType` (`playwright.chromium`, `firefox`, `webkit`), mapped onto Neo's
 * attached browser. A run never launches or connects to a browser: it is already attached to
 * BrowserOS, which `context.browser()` (or `playwright.chromium`'s `browser()`) reaches. So
 * `name()` answers, and every launch or connect method refuses with an Error that says where the
 * run's browser is, instead of starting an unrelated one. Firefox and WebKit are not Neo's.
 *
 * Deviation: `client/browserType.ts` `BrowserType.launch`: `const browser = Browser.from((await this._channel.launch(launchOptions, new TimeoutSettings().launchTimeout(options))).browser);`
 * Neo runs attached to the user's BrowserOS and never starts a browser, so `launch`,
 * `launchPersistentContext`, `launchServer`, `connect` and `connectOverCDP` reject, before any
 * browser effect.
 */

import type { Browser } from './browser'

/** Neo: why a run cannot launch or connect: it already has its browser. */
const ATTACHED_REFUSAL =
  'Neo runs attached to BrowserOS; use the provided context / context.browser()'

/** Neo: why Firefox and WebKit refuse: Neo drives BrowserOS, a Chromium browser. */
const CHROMIUM_ONLY_REFUSAL =
  'Neo drives BrowserOS (Chromium) only; firefox and webkit are not available'

/**
 * Deviation: `client/browserType.ts` `BrowserType.executablePath`: `throw new Error('Browser is not supported on current platform');`
 * Playwright answers the path of the browser build it would launch. Neo launches none, and CDP
 * does not report the attached BrowserOS's own path (`Browser.getBrowserCommandLine` answers
 * only under `--enable-automation`), so it throws, as Playwright does where it has no path.
 */
const NO_EXECUTABLE_PATH =
  'Neo runs attached to BrowserOS and launches no browser; CDP does not report its executable path'

export type BrowserName = 'chromium' | 'firefox' | 'webkit'

export class BrowserType {
  readonly #name: BrowserName

  constructor(name: BrowserName) {
    this.#name = name
  }

  name(): string {
    return this.#name
  }

  executablePath(): string {
    throw new Error(NO_EXECUTABLE_PATH)
  }

  async launch(_options?: unknown): Promise<Browser> {
    this.#refuse('browserType.launch')
  }

  async launchPersistentContext(
    _userDataDir?: string,
    _options?: unknown,
  ): Promise<never> {
    this.#refuse('browserType.launchPersistentContext')
  }

  async launchServer(_options?: unknown): Promise<never> {
    this.#refuse('browserType.launchServer')
  }

  async connect(
    _optionsOrEndpoint?: unknown,
    _options?: unknown,
  ): Promise<Browser> {
    this.#refuse('browserType.connect')
  }

  async connectOverCDP(
    _endpointOrOptions?: unknown,
    _options?: unknown,
  ): Promise<Browser> {
    this.#refuse('browserType.connectOverCDP')
  }

  /** Playwright's `ChannelOwner.toJSON` shape for a browser type. */
  toJSON(): { _type: 'BrowserType'; _guid: string } {
    return { _type: 'BrowserType', _guid: `browser-type@${this.#name}` }
  }

  /** Neo: the refusal every launch and connect method gives, prefixed with its API name. */
  #refuse(apiName: string): never {
    throw new Error(
      `${apiName}: ${this.#name === 'chromium' ? ATTACHED_REFUSAL : CHROMIUM_ONLY_REFUSAL}`,
    )
  }
}
