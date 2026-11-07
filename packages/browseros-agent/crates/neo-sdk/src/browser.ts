/**
 * Playwright: `client/browser.ts` `Browser`
 *
 * Playwright's `Browser` for the BrowserOS a run is attached to (`browser@default`), which
 * `context.browser()` returns. It behaves as Playwright's connected browser
 * (`chromium.connectOverCDP`): `contexts()` are the run's own context plus the contexts this run
 * created, `newContext` creates an isolated one, and `close()` closes the contexts it created and
 * then reads disconnected, every later call failing as on a closed browser. BrowserOS itself is
 * never closed: it is the user's browser.
 *
 * Deviation: `client/browser.ts` `Browser.close`: `await this._channel.close(options, kNoTimeout);`
 * Playwright's connected `close` ends its connection; Neo's run keeps its CDP connection (the
 * runtime's, shared by every run), so only this run's objects read disconnected.
 */

import type {
  BrowserCloseParams,
  BrowserNewContextParams,
} from '@browseros/neo-runtime-spec/serve'
import { BrowserContext } from './browser-context'
import type { BrowserType } from './browser-type'
import type { Connection } from './connection'
import { ChannelOwner, type EventPipe, type EventTable } from './events'
import type { Page } from './page'
import { pickParams } from './protocol'
import { get } from './runtime-bridge'

/**
 * Neo: the browser's listener events (`Events.Browser`). `context` and `disconnected` are
 * emitted by this object itself, as Playwright's client does on its own `context` and `close`
 * handlers, so neither needs a subscription.
 */
const BROWSER_EVENTS: EventTable = {
  receiver: 'Browser',
  served: [],
  quiet: [],
  unserved: [],
}

/** Playwright: `client/browser.ts` `Browser.close`: `await this._channel.close(options, kNoTimeout);` */
const BROWSER_CLOSE_KEYS = ['reason'] as const

/**
 * `Browser.newContext`'s params in Playwright's schema order (`spec/playwright/browser.yml`,
 * its `ContextOptions` mixin first).
 */
const BROWSER_NEW_CONTEXT_KEYS = [
  'noDefaultViewport',
  'viewport',
  'screen',
  'ignoreHTTPSErrors',
  'clientCertificates',
  'javaScriptEnabled',
  'bypassCSP',
  'userAgent',
  'locale',
  'timezoneId',
  'geolocation',
  'permissions',
  'extraHTTPHeaders',
  'offline',
  'httpCredentials',
  'deviceScaleFactor',
  'isMobile',
  'hasTouch',
  'colorScheme',
  'reducedMotion',
  'forcedColors',
  'acceptDownloads',
  'contrast',
  'baseURL',
  'recordVideo',
  'strictSelectors',
  'serviceWorkers',
  'selectorEngines',
  'testIdAttributeName',
  'proxy',
  'storageState',
] as const

type SameKeys<A, B> = [keyof A] extends [keyof B]
  ? [keyof B] extends [keyof A]
    ? true
    : false
  : false
type Assert<T extends true> = T
type _BrowserNewContextKeysMatchSpec = Assert<
  SameKeys<
    Record<(typeof BROWSER_NEW_CONTEXT_KEYS)[number], true>,
    Required<BrowserNewContextParams>
  >
>

/**
 * Deviation: `client/browser.ts` `Browser._innerNewContext`: `context._logger = options.logger;`
 * Playwright's client logs each API call of a context created with `logger`, and records a HAR
 * for `recordHar`, both in its own process. Neo records every call in the run's outcome instead
 * and serves neither option yet, so each refuses by name.
 */
const CLIENT_OPTIONS = ['logger', 'recordHar'] as const

/**
 * Deviation: `client/browserContext.ts` `prepareBrowserContextParams`: `storageState: options.storageState ? await prepareStorageState(options.storageState) : undefined,`
 * A storage state file path needs the host's filesystem, which a run's realm has none of; the
 * refusal is the one neo-runtime gives a storage state object.
 */
const STORAGE_STATE_REFUSAL =
  'Neo does not support the "storageState" option of a new context yet'

/** Playwright's `BrowserContextOptions` as a script passes them to `newContext`. */
export type BrowserContextOptions = {
  viewport?: { width: number; height: number } | null
  screen?: { width: number; height: number }
  deviceScaleFactor?: number
  isMobile?: boolean
  hasTouch?: boolean
  userAgent?: string
  locale?: string
  timezoneId?: string
  colorScheme?: 'light' | 'dark' | 'no-preference' | null
  reducedMotion?: 'reduce' | 'no-preference' | null
  forcedColors?: 'active' | 'none' | null
  contrast?: 'no-preference' | 'more' | null
  javaScriptEnabled?: boolean
  bypassCSP?: boolean
  ignoreHTTPSErrors?: boolean
  acceptDownloads?: boolean
  extraHTTPHeaders?: Record<string, string>
  httpCredentials?: unknown
  storageState?: unknown
  [option: string]: unknown
}

export class Browser extends ChannelOwner {
  readonly #connection: Connection
  readonly #pipe: EventPipe
  readonly #browserType: BrowserType
  /** Playwright's `Browser._contexts`: the run's own context, then those this run created. */
  readonly _contexts = new Set<BrowserContext>()
  #isConnected = true
  _closeReason: string | undefined

  constructor(
    guid: string,
    connection: Connection,
    pipe: EventPipe,
    browserType: BrowserType,
  ) {
    super(guid, pipe, BROWSER_EVENTS)
    this.#connection = connection
    this.#pipe = pipe
    this.#browserType = browserType
  }

  /** Neo's browser sends no channel events: `context` and `disconnected` come from its calls. */
  _onEvent(): void {}

  browserType(): BrowserType {
    return this.#browserType
  }

  contexts(): BrowserContext[] {
    return [...this._contexts]
  }

  /** The attached browser's version, as Playwright's connected browser reports it. */
  version(): string {
    return get(this._guid, 'version')
  }

  /** `false` once `close()` ran, or while the runtime's connection to BrowserOS is down. */
  isConnected(): boolean {
    return this.#isConnected && get(this._guid, 'isConnected')
  }

  /**
   * Playwright's `Browser.newContext`: a fresh isolated context, sharing no cookies or storage
   * with the run's own context. Its pages emulate the options Neo serves (viewport, screen,
   * device scale factor, mobile, touch, user agent, locale, time zone, media features,
   * JavaScript, CSP bypass, HTTPS errors); any other option is refused by name.
   *
   * Deviation: `server/chromium/crBrowser.ts` `CRBrowserContext.doClose`: `await this._browser._session.send('Target.disposeBrowserContext', { browserContextId: this._browserContextId });`
   * Playwright keeps the context until it is closed; Neo disposes it, with its pages, when the
   * run ends.
   */
  async newContext(
    options: BrowserContextOptions = {},
  ): Promise<BrowserContext> {
    for (const option of CLIENT_OPTIONS)
      if (options[option] !== undefined)
        throw new Error(
          `browser.newContext: Neo does not support the "${option}" option of a new context yet`,
        )
    if (typeof options.storageState === 'string')
      throw new Error(`browser.newContext: ${STORAGE_STATE_REFUSAL}`)
    const params = pickParams(
      BROWSER_NEW_CONTEXT_KEYS,
      prepareBrowserContextParams(options),
    ) as BrowserNewContextParams
    const { context } = await this.#connection.sendBrowser(
      'browser.newContext',
      this._guid,
      'newContext',
      params,
      0,
    )
    const created = new BrowserContext(
      context.guid,
      this.#connection,
      this.#pipe,
    )
    this._didCreateContext(created)
    return created
  }

  /**
   * Playwright's `Browser.newPage`: a page in a context of its own, which closes with the page.
   */
  async newPage(options: BrowserContextOptions = {}): Promise<Page> {
    const context = await this.newContext(options)
    const page = await context.newPage()
    page._ownedContext = context
    context._ownerPage = page
    return page
  }

  /**
   * Playwright's connected `close`: every context this run created closes with its pages, then
   * every object of the run reads closed (`isConnected()` is false, `contexts()` empty, each
   * page's `isClosed()` true) and the `disconnected` event fires. Calls after it fail with
   * Playwright's closed-target error. The run's own tabs stay open in BrowserOS.
   */
  async close(options: { reason?: string } = {}): Promise<void> {
    this._closeReason = options.reason
    if (!this.#isConnected) return
    // The pages open now are the ones that close: a created context's pages are gone once
    // Rust answers, so they are read before.
    const open = this.contexts().map((context) => context._openPages())
    await this.#connection.sendBrowser(
      'browser.close',
      this._guid,
      'close',
      pickParams(BROWSER_CLOSE_KEYS, {
        reason: options.reason,
      }) as BrowserCloseParams,
      0,
    )
    this._didClose(open)
  }

  /** Playwright's `Browser._didClose`, in the order Playwright's connected browser fires it:
   * every open page, then each created context, then the run's own, then `disconnected`. */
  _didClose(open: Page[][]): void {
    this.#isConnected = false
    this.#connection._disconnect()
    this.#pipe._disconnect()
    const contexts = this.contexts()
    for (const pages of open) for (const page of pages) page._onClose()
    for (const context of contexts.slice(1)) context._onClose()
    contexts[0]?._onClose()
    this.emit('disconnected', this)
  }

  /** Neo: the `Page` any of the run's contexts holds for `guid`. */
  _knownPage(guid: string): Page | undefined {
    for (const context of this._contexts) {
      const page = context._knownPage(guid)
      if (page) return page
    }
    return undefined
  }

  /** Playwright's `Browser._didCreateContext`. */
  _didCreateContext(context: BrowserContext): void {
    context._browser = this
    this._contexts.add(context)
    this.emit('context', context)
  }

  /**
   * Deviation: `client/browser.ts` `Browser.bind`: `const { endpoint } = await this._channel.startServer({ title, ...options }, kNoTimeout);`
   * `bind` serves the browser to other Playwright clients (its CLI and MCP tools) over a socket
   * Playwright opens; a Neo run is the only client of its browser and opens no server.
   */
  async bind(
    _title: string,
    _options?: unknown,
  ): Promise<{ endpoint: string }> {
    throw new Error(
      'browser.bind: Neo runs inside BrowserOS and serves the browser to no other client',
    )
  }

  /** The other half of {@link Browser.bind}, refused as it is. */
  async unbind(): Promise<void> {
    throw new Error(
      'browser.unbind: Neo runs inside BrowserOS and serves the browser to no other client',
    )
  }

  /** Neo: raw CDP is a separate, guarded slice; until it lands this refuses. */
  async newBrowserCDPSession(): Promise<never> {
    throw new Error(
      'browser.newBrowserCDPSession: Neo does not serve CDP sessions yet',
    )
  }

  /** Neo: browser tracing is a separate slice; until it lands this refuses. */
  async startTracing(_page?: Page, _options?: unknown): Promise<void> {
    throw new Error('browser.startTracing: Neo does not serve tracing yet')
  }

  /** Neo: browser tracing is a separate slice; until it lands this refuses. */
  async stopTracing(): Promise<never> {
    throw new Error('browser.stopTracing: Neo does not serve tracing yet')
  }

  async [Symbol.asyncDispose](): Promise<void> {
    await this.close()
  }

  /** Playwright's `ChannelOwner.toJSON`: the browser's type and guid only. */
  toJSON(): { _type: 'Browser'; _guid: string } {
    return { _type: 'Browser', _guid: this._guid }
  }
}

/** Playwright: `client/browserContext.ts` `prepareBrowserContextParams` */
function prepareBrowserContextParams(
  options: BrowserContextOptions,
): Record<string, unknown> {
  return {
    ...options,
    viewport: options.viewport === null ? undefined : options.viewport,
    noDefaultViewport: options.viewport === null,
    extraHTTPHeaders: options.extraHTTPHeaders
      ? headersObjectToArray(options.extraHTTPHeaders)
      : undefined,
    colorScheme:
      options.colorScheme === null ? 'no-override' : options.colorScheme,
    reducedMotion:
      options.reducedMotion === null ? 'no-override' : options.reducedMotion,
    forcedColors:
      options.forcedColors === null ? 'no-override' : options.forcedColors,
    contrast: options.contrast === null ? 'no-override' : options.contrast,
    acceptDownloads: toAcceptDownloadsProtocol(options.acceptDownloads),
    httpCredentials: toHttpCredentialsProtocol(options.httpCredentials),
  }
}

/** Playwright: `client/browserContext.ts` `toAcceptDownloadsProtocol` */
function toAcceptDownloadsProtocol(
  acceptDownloads?: boolean,
): BrowserNewContextParams['acceptDownloads'] {
  if (acceptDownloads === undefined) return undefined
  if (acceptDownloads) return 'accept'
  return 'deny'
}

/** Playwright: `client/browserContext.ts` `toHttpCredentialsProtocol` */
function toHttpCredentialsProtocol(
  credentials?: unknown,
): unknown[] | undefined {
  if (!credentials) return undefined
  const list = Array.isArray(credentials) ? credentials : [credentials]
  return list.length ? list : undefined
}

/** Playwright: `isomorphic/headers.ts` `headersObjectToArray` */
function headersObjectToArray(
  headers: Record<string, string>,
): { name: string; value: string }[] {
  const result: { name: string; value: string }[] = []
  for (const name in headers) {
    if (!Object.is(headers[name], undefined))
      result.push({ name, value: headers[name] as string })
  }
  return result
}
