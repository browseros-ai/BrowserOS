/**
 * Playwright: `client/network.ts` `Route`
 * Playwright: `client/network.ts` `RouteHandler`
 *
 * A request a route paused, and the handlers that answer it (`page.route`, `context.route`).
 * While the run has a route, Rust pauses every request of its tabs and hands each one a
 * pattern names to the page or to the context as a `route` event (neo-runtime `routes.rs`);
 * the page's handlers run first, newest first, then the context's, then the request goes on
 * with what their `fallback()` calls changed. A handler's `abort`, `continue` or `fulfill`
 * answers it once on its `route@<targetId>:<n>` guid.
 */

import type { ApiName, Connection } from './connection'
import { isTargetClosedError } from './errors'
import type { NeoSerializedUrlMatch } from './neo-commands'
import { isRegExp } from './neo-commands'
import {
  encodeBase64,
  encodeUtf8,
  type FallbackOverrides,
  type Headers,
  type Request,
} from './network'
import { matchesURL } from './runtime-bridge'
import { type URLPredicate, urlFor } from './url'
import { rewriteErrorMessage } from './waiter'

/** Playwright's `URLMatch` without `URLPattern`, which QuickJS lacks. */
export type RouteURLMatch = string | RegExp | URLPredicate

/** Playwright's `RouteHandlerCallback`. */
export type RouteHandlerCallback = (route: Route, request: Request) => unknown

/** Playwright's `route.fulfill` options. */
export type FulfillOptions = {
  response?: unknown
  status?: number
  headers?: Headers
  contentType?: string
  body?: string | Uint8Array
  json?: unknown
  path?: string
}

/** Playwright's `page.unrouteAll` options. */
export type UnrouteBehavior = 'wait' | 'ignoreErrors' | 'default'

/**
 * Deviation: `client/network.ts` `Route._innerFulfill`: `body = body.toString('base64');`
 * A body crosses to Rust in base64 inside one call, which the run's 8 MiB of requests in
 * flight bounds (neo-runtime `dispatch.rs`); Neo refuses a larger body by name rather than
 * with that cap's message.
 */
const MAX_FULFILL_BYTES = 6 * 1024 * 1024

/** Neo: the refusal of a body past {@link MAX_FULFILL_BYTES}. */
const BODY_TOO_LARGE =
  'route.fulfill: Neo carries a body of at most 6291456 bytes (6 MiB) to the browser'

/** Playwright: `client/network.ts` `Route._checkNotHandled`: `throw new Error('Route is already handled!');` */
const ROUTE_HANDLED = 'Route is already handled!'

/** Playwright: `client/network.ts` `Route._innerFulfill`: `assert(options.body === undefined, 'Can specify either body or json parameters');` */
const BODY_AND_JSON = 'Can specify either body or json parameters'

/**
 * Deviation: `client/network.ts` `Route._innerFulfill`: `const buffer = await fs.promises.readFile(options.path);`
 * Neo reads files only inside the run's artifact directory, which med-pw-eval's path seam
 * (#3182) brings; until then a `path` is refused by name.
 */
const NO_PATH =
  'route.fulfill: Neo cannot read a file for "path" yet (it reads only inside the run\'s artifact directory, which this build lacks); pass "body" instead'

/** Neo: a promise resolved from outside, as Playwright's `ManualPromise` is used here. */
function deferred<T>(): { promise: Promise<T>; resolve: (value: T) => void } {
  let resolve: (value: T) => void = () => {}
  const promise = new Promise<T>((done) => {
    resolve = done
  })
  return { promise, resolve }
}

/** Playwright: `isomorphic/headers.ts` `headersObjectToArray`: `if (values === undefined)` */
function headersObjectToArray(
  headers: Headers,
): { name: string; value: string }[] {
  return Object.entries(headers)
    .filter(([, value]) => value !== undefined)
    .map(([name, value]) => ({ name, value }))
}

/** Playwright: `isomorphic/urlMatch.ts` `serializeURLMatch`: `// Functions cannot be serialized` */
function serializeURLMatch(match: RouteURLMatch): NeoSerializedUrlMatch | null {
  if (typeof match === 'string') return { glob: match }
  if (isRegExp(match))
    return { regexSource: match.source, regexFlags: match.flags }
  return null
}

/** Playwright: `isomorphic/urlMatch.ts` `urlMatchesEqual`: `return match1.source === match2.source && match1.flags === match2.flags;` */
export function urlMatchesEqual(
  match1: RouteURLMatch,
  match2: RouteURLMatch,
): boolean {
  if (isRegExp(match1) && isRegExp(match2))
    return match1.source === match2.source && match1.flags === match2.flags
  return match1 === match2
}

/** Playwright: `client/network.ts` `Route` */
export class Route {
  readonly _guid: string
  readonly #request: Request
  readonly #connection: Connection
  #handlingPromise: ReturnType<typeof deferred<boolean>> | null = null
  _didThrow = false

  constructor(guid: string, request: Request, connection: Connection) {
    this._guid = guid
    this.#request = request
    this.#connection = connection
  }

  request(): Request {
    return this.#request
  }

  /**
   * Deviation: `client/network.ts` `Route._raceWithTargetClose`: `return await this.request()._targetClosedScope().safeRace(promise);`
   * Playwright resolves the call as soon as the page closes; Neo's call itself ends with
   * `TargetClosedError` when the tab goes, which this turns into the same quiet resolve.
   */
  async #raceWithTargetClose(promise: Promise<unknown>): Promise<void> {
    try {
      await promise
    } catch (error) {
      if (isTargetClosedError(error)) return
      throw error
    }
  }

  _startHandling(): Promise<boolean> {
    this.#handlingPromise = deferred<boolean>()
    return this.#handlingPromise.promise
  }

  async fallback(options: FallbackOverrides = {}): Promise<void> {
    this._checkNotHandled()
    this.#request._applyFallbackOverrides(options)
    this._reportHandled(false)
  }

  async abort(errorCode?: string): Promise<void> {
    await this.#handleRoute(async () => {
      await this.#raceWithTargetClose(
        this.#connection.sendRoute(
          'route.abort',
          this._guid,
          'abort',
          { errorCode },
          0,
        ),
      )
    })
  }

  async fulfill(options: FulfillOptions = {}): Promise<void> {
    await this.#handleRoute(async () => {
      await this.#innerFulfill(options)
    })
  }

  async #handleRoute(callback: () => Promise<void>): Promise<void> {
    this._checkNotHandled()
    try {
      await callback()
      this._reportHandled(true)
    } catch (error) {
      this._didThrow = true
      throw error
    }
  }

  async #innerFulfill(options: FulfillOptions): Promise<void> {
    let { status: statusOption, headers: headersOption, body } = options
    if (options.json !== undefined) {
      if (options.body !== undefined) throw new Error(BODY_AND_JSON)
      body = JSON.stringify(options.json)
    }
    if (options.path !== undefined) throw new Error(NO_PATH)
    let isBase64 = false
    let length = 0
    let wire: string | undefined
    if (typeof body === 'string') {
      length = encodeUtf8(body).length
      wire = body
    } else if (body instanceof Uint8Array) {
      length = body.length
      wire = encodeBase64(body)
      isBase64 = true
    }
    if (length > MAX_FULFILL_BYTES) throw new Error(BODY_TOO_LARGE)
    const headers: Headers = {}
    for (const header of Object.keys(headersOption || {}))
      headers[header.toLowerCase()] = String((headersOption as Headers)[header])
    if (options.contentType)
      headers['content-type'] = String(options.contentType)
    else if (options.json) headers['content-type'] = 'application/json'
    if (length && !('content-length' in headers))
      headers['content-length'] = String(length)
    await this.#raceWithTargetClose(
      this.#connection.sendRoute(
        'route.fulfill',
        this._guid,
        'fulfill',
        {
          status: statusOption || 200,
          headers: headersObjectToArray(headers),
          body: wire,
          isBase64,
        },
        0,
      ),
    )
  }

  async continue(options: FallbackOverrides = {}): Promise<void> {
    await this.#handleRoute(async () => {
      this.#request._applyFallbackOverrides(options)
      await this._innerContinue(false)
    })
  }

  _checkNotHandled(): void {
    if (!this.#handlingPromise) throw new Error(ROUTE_HANDLED)
  }

  _reportHandled(done: boolean): void {
    const chain = this.#handlingPromise
    this.#handlingPromise = null
    chain?.resolve(done)
  }

  async _innerContinue(isFallback: boolean): Promise<void> {
    const options = this.#request._fallbackOverridesForContinue()
    await this.#raceWithTargetClose(
      this.#connection.sendRoute(
        'route.continue',
        this._guid,
        'continue',
        {
          url: options.url,
          method: options.method,
          headers: options.headers
            ? headersObjectToArray(options.headers)
            : undefined,
          postData: options.postDataBuffer
            ? encodeBase64(options.postDataBuffer)
            : undefined,
          isFallback,
        },
        0,
      ),
    )
  }
}

/** Playwright: `client/network.ts` `RouteHandler` */
export class RouteHandler {
  #handledCount = 0
  readonly #times: number
  readonly url: RouteURLMatch
  readonly handler: RouteHandlerCallback
  #ignoreException = false
  readonly #activeInvocations = new Set<{
    complete: ReturnType<typeof deferred<void>>
    route: Route
  }>()

  constructor(
    url: RouteURLMatch,
    handler: RouteHandlerCallback,
    times: number = Number.MAX_SAFE_INTEGER,
  ) {
    this.#times = times
    this.url = url
    this.handler = handler
    // Playwright validates a string glob at once, so a bad pattern throws at `route()`.
    if (typeof url === 'string') matchesURL('about:blank', { glob: url })
  }

  static prepareInterceptionPatterns(
    handlers: RouteHandler[],
  ): NeoSerializedUrlMatch[] {
    const patterns: NeoSerializedUrlMatch[] = []
    let all = false
    for (const handler of handlers) {
      const serialized = serializeURLMatch(handler.url)
      if (serialized) patterns.push(serialized)
      else all = true
    }
    if (all) return [{ glob: '**/*' }]
    return patterns
  }

  matches(requestURL: string): boolean {
    if (typeof this.url === 'function') {
      const url = urlFor(requestURL)
      return !!url && !!this.url(url)
    }
    const serialized = serializeURLMatch(this.url)
    return !!serialized && matchesURL(requestURL, serialized)
  }

  async handle(route: Route): Promise<boolean> {
    const invocation = { complete: deferred<void>(), route }
    this.#activeInvocations.add(invocation)
    try {
      return await this.#handleInternal(route)
    } catch (error) {
      // A handler stopped without waiting for it has its exceptions ignored.
      if (this.#ignoreException) return false
      if (isTargetClosedError(error) && error instanceof Error)
        rewriteErrorMessage(
          error,
          `"${error.message}" while running route callback.\nConsider awaiting \`await page.unrouteAll({ behavior: 'ignoreErrors' })\`\nbefore the end of the test to ignore remaining routes in flight.`,
        )
      throw error
    } finally {
      invocation.complete.resolve()
      this.#activeInvocations.delete(invocation)
    }
  }

  async stop(behavior: Exclude<UnrouteBehavior, 'default'>): Promise<void> {
    if (behavior === 'ignoreErrors') {
      this.#ignoreException = true
      return
    }
    const promises: Promise<void>[] = []
    for (const activation of this.#activeInvocations)
      if (!activation.route._didThrow)
        promises.push(activation.complete.promise)
    await Promise.all(promises)
  }

  async #handleInternal(route: Route): Promise<boolean> {
    ++this.#handledCount
    const handledPromise = route._startHandling()
    const handler = this.handler
    const [handled] = await Promise.all([
      handledPromise,
      handler(route, route.request()),
    ])
    return handled
  }

  willExpire(): boolean {
    return this.#handledCount + 1 >= this.#times
  }
}

/** Neo: the api name a route call logs under. */
export type RouteApiName = Extract<ApiName, `route.${string}`>
