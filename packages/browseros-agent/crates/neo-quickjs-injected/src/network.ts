/**
 * Playwright's client `Request`, `Response` and `RawHeaders` (`client/network.ts` at 26137face),
 * for every request of a page the run recorded: the objects behind `page.goto`'s answer (the
 * navigation's request, one per redirect, and each one's response), the run's `request`,
 * `response`, `requestfinished` and `requestfailed` events, `waitForRequest`/`waitForResponse`
 * and `page.requests()`. Rust records them from the tab's Network events (neo-runtime
 * `network.rs`) and mints their guids; `networkObject` builds a hop with every hop before it from
 * Rust's `chain` getter, and an event's own record builds its object directly, once per guid,
 * through the connection's object map, so a guid is always one object, whichever way the script
 * met it first. Each hop's `frame()` is the page's frame its record names.
 *
 * Playwright's client keeps some of this state current from events (`requestFinished`,
 * `requestFailed`, `response`, a redirect's `request`): `timing().responseEnd`, `failure()`,
 * `finished()`, `existingResponse()` and `redirectedTo()`. Neo produces an event only while the
 * script listens, so those read Rust's record live, through getters, and can be a step ahead of
 * the events the script has been handed.
 *
 * What differs from Playwright:
 * - `body()` resolves a `Uint8Array` subclass named `Buffer` with Node's `toString(encoding)`,
 *   `toJSON()` and `equals()` (`buffer.ts`); QuickJS has no `Buffer`, and the SDK defines no
 *   globals.
 * - `finished()` polls Rust's record (every few to 100 ms) instead of waiting for an event.
 * - `serviceWorker()` is always `null`: Neo records no worker's requests.
 */

import type {
  NameValue,
  RemoteAddr,
  RequestInitializer,
  RequestSizes,
  ResourceTiming,
  ResponseInitializer,
  SecurityDetails,
} from '@browseros/neo-runtime-spec/serve'
import { Buffer } from './buffer'
import type { ApiName, Connection } from './connection'
import { targetClosedError } from './errors'
import type { Frame } from './frame'
import type { Page } from './page'
import { get } from './runtime-bridge'

/** Playwright's `Headers`: lower-cased names to joined values. */
export type Headers = { [name: string]: string }

/** `request.timing()`: the protocol's `ResourceTiming` plus the client's `responseEnd`. */
export type Timing = ResourceTiming & { responseEnd: number }

/** One object Rust created, as its `chain` getter answers it. */
export type Created<Initializer> = { guid: string; initializer: Initializer }

/** Rust's `chain` getter on a response: every hop up to it, oldest first. */
export type CreatedChain = {
  requests: Created<RequestInitializer>[]
  /** One per request; `null` for a hop that has no response yet. */
  responses: (Created<ResponseInitializer> | null)[]
}

/** Playwright's `RawHeaders` (`client/network.ts`). */
/** Playwright's `FallbackOverrides` (`client/network.ts`): what a route's `fallback()` or
 * `continue()` may change. */
export type FallbackOverrides = {
  url?: string
  method?: string
  headers?: Headers
  postData?: unknown
}

/** Playwright's `SerializedFallbackOverrides`. */
export type SerializedFallbackOverrides = {
  url?: string
  method?: string
  headers?: Headers
  postDataBuffer?: Buffer
}

export class RawHeaders {
  readonly #headersArray: NameValue[]
  readonly #headersMap = new Map<string, string[]>()

  constructor(headers: NameValue[]) {
    this.#headersArray = headers
    for (const header of headers) {
      const name = header.name.toLowerCase()
      const values = this.#headersMap.get(name)
      if (values) values.push(header.value)
      else this.#headersMap.set(name, [header.value])
    }
  }

  /** Playwright's `RawHeaders._fromHeadersObjectLossy`: an object's headers, `undefined` values
   * left out. */
  static _fromHeadersObjectLossy(headers: Headers): RawHeaders {
    return new RawHeaders(
      Object.entries(headers)
        .filter(([, value]) => value !== undefined)
        .map(([name, value]) => ({ name, value })),
    )
  }

  get(name: string): string | null {
    const values = this.getAll(name)
    if (!values.length) return null
    return values.join(name.toLowerCase() === 'set-cookie' ? '\n' : ', ')
  }

  getAll(name: string): string[] {
    return [...(this.#headersMap.get(name.toLowerCase()) ?? [])]
  }

  headers(): Headers {
    const result: Headers = {}
    for (const name of this.#headersMap.keys())
      result[name] = this.get(name) ?? ''
    return result
  }

  headersArray(): NameValue[] {
    return this.#headersArray
  }
}

export class Request {
  readonly _guid: string
  readonly #initializer: RequestInitializer
  readonly #frame: Frame
  readonly #connection: Connection
  readonly #redirectedFrom: Request | null
  _redirectedTo: Request | null = null
  _response: Response | null = null
  readonly #provisionalHeaders: RawHeaders
  #actualHeadersPromise: Promise<RawHeaders> | undefined
  /** Playwright's `_fallbackOverrides`: what a route's `fallback()` and `continue()` changed,
   * which this request then reports and the final continue sends. */
  readonly #fallbackOverrides: SerializedFallbackOverrides = {}
  /** Playwright's `_timing`: one object, refreshed from Rust on every `timing()` read. */
  readonly #timing: Timing = {
    startTime: 0,
    domainLookupStart: -1,
    domainLookupEnd: -1,
    connectStart: -1,
    secureConnectionStart: -1,
    connectEnd: -1,
    requestStart: -1,
    responseStart: -1,
    responseEnd: -1,
  }

  constructor(
    guid: string,
    initializer: RequestInitializer,
    frame: Frame,
    redirectedFrom: Request | null,
    connection: Connection,
  ) {
    this._guid = guid
    this.#initializer = initializer
    this.#frame = frame
    this.#connection = connection
    this.#redirectedFrom = redirectedFrom
    if (redirectedFrom) redirectedFrom._redirectedTo = this
    this.#provisionalHeaders = new RawHeaders(initializer.headers)
  }

  url(): string {
    return this.#fallbackOverrides.url || this.#initializer.url
  }

  resourceType(): string {
    return this.#initializer.resourceType
  }

  method(): string {
    return this.#fallbackOverrides.method || this.#initializer.method
  }

  postData(): string | null {
    return this.postDataBuffer()?.toString('utf-8') ?? null
  }

  postDataBuffer(): Buffer | null {
    if (this.#fallbackOverrides.postDataBuffer)
      return this.#fallbackOverrides.postDataBuffer
    const data = this.#initializer.postData
    return data ? Buffer.fromBinary(data) : null
  }

  postDataJSON(): object | null {
    const postData = this.postData()
    if (!postData) return null
    const contentType = this.headers()['content-type']
    if (contentType?.includes('application/x-www-form-urlencoded'))
      return parseFormBody(postData)
    try {
      return JSON.parse(postData)
    } catch {
      throw new Error(`POST data is not a valid JSON object: ${postData}`)
    }
  }

  /** The provisional headers, as Playwright's deprecated `headers()`; a route's header
   * overrides once it gave some. */
  headers(): Headers {
    if (this.#fallbackOverrides.headers)
      return RawHeaders._fromHeadersObjectLossy(
        this.#fallbackOverrides.headers,
      ).headers()
    return this.#provisionalHeaders.headers()
  }

  /** Playwright's `Request._applyFallbackOverrides`: a route's overrides, kept for the final
   * continue; headers replace the whole set. */
  _applyFallbackOverrides(overrides: FallbackOverrides): void {
    if (overrides.url) this.#fallbackOverrides.url = overrides.url
    if (overrides.method) this.#fallbackOverrides.method = overrides.method
    if (overrides.headers) this.#fallbackOverrides.headers = overrides.headers
    if (typeof overrides.postData === 'string')
      this.#fallbackOverrides.postDataBuffer = encodeUtf8(overrides.postData)
    else if (overrides.postData instanceof Uint8Array) {
      const bytes = new Buffer(overrides.postData.length)
      bytes.set(overrides.postData)
      this.#fallbackOverrides.postDataBuffer = bytes
    } else if (overrides.postData)
      this.#fallbackOverrides.postDataBuffer = encodeUtf8(
        JSON.stringify(overrides.postData),
      )
  }

  /** Playwright's `Request._fallbackOverridesForContinue`. */
  _fallbackOverridesForContinue(): SerializedFallbackOverrides {
    return this.#fallbackOverrides
  }

  async allHeaders(): Promise<Headers> {
    return (await this.#actualHeaders('request.allHeaders')).headers()
  }

  async headersArray(): Promise<NameValue[]> {
    return (await this.#actualHeaders('request.headersArray')).headersArray()
  }

  async headerValue(name: string): Promise<string | null> {
    return (await this.#actualHeaders('request.headerValue')).get(name)
  }

  async response(): Promise<Response | null> {
    return await this.#response('request.response')
  }

  /** The response once it arrived, as Rust's record has it now. */
  existingResponse(): Response | null {
    if (this._response) return this._response
    const guid = get(this._guid, 'existingResponse')
    return guid
      ? (networkObject(this.#connection, guid, this.#frame._page) as Response)
      : null
  }

  frame(): Frame {
    return this.#frame
  }

  serviceWorker(): null {
    return null
  }

  isNavigationRequest(): boolean {
    return this.#initializer.isNavigationRequest
  }

  redirectedFrom(): Request | null {
    return this.#redirectedFrom
  }

  /** The request a redirect made of this one, once it started, as Rust's record has it now. */
  redirectedTo(): Request | null {
    if (this._redirectedTo) return this._redirectedTo
    const guid = get(this._guid, 'redirectedTo')
    return guid
      ? (networkObject(this.#connection, guid, this.#frame._page) as Request)
      : null
  }

  /** Playwright's `_failureText = failureText || null`: an empty text reads as no failure. */
  failure(): { errorText: string } | null {
    const errorText = get(this._guid, 'failure')
    return errorText ? { errorText } : null
  }

  timing(): Timing {
    return Object.assign(this.#timing, get(this._guid, 'timing'))
  }

  async sizes(): Promise<RequestSizes> {
    const response = await this.#response('request.sizes')
    if (!response) throw new Error('Unable to fetch sizes for failed request')
    return await response._sizes('request.sizes')
  }

  toJSON(): { _type: 'Request'; _guid: string } {
    return { _type: 'Request', _guid: this._guid }
  }

  async #response(apiName: ApiName): Promise<Response | null> {
    const { response } = await this.#connection.sendRequest(
      apiName,
      this._guid,
      'response',
      {},
      0,
    )
    return response
      ? (networkObject(
          this.#connection,
          response.guid,
          this.#frame._page,
        ) as Response)
      : null
  }

  /** Playwright's `Request._actualHeaders`: a route's header overrides once it gave some, else
   * the headers the browser sent. */
  #actualHeaders(apiName: ApiName): Promise<RawHeaders> {
    if (this.#fallbackOverrides.headers)
      return Promise.resolve(
        RawHeaders._fromHeadersObjectLossy(this.#fallbackOverrides.headers),
      )
    this.#actualHeadersPromise ??= this.#connection
      .sendRequest(apiName, this._guid, 'rawRequestHeaders', {}, 0)
      .then(({ headers }) => new RawHeaders(headers))
    return this.#actualHeadersPromise
  }
}

export class Response {
  readonly _guid: string
  readonly #initializer: ResponseInitializer
  readonly #request: Request
  readonly #connection: Connection
  readonly #provisionalHeaders: RawHeaders
  #actualHeadersPromise: Promise<RawHeaders> | undefined

  constructor(
    guid: string,
    initializer: ResponseInitializer,
    request: Request,
    connection: Connection,
  ) {
    this._guid = guid
    this.#initializer = initializer
    this.#request = request
    this.#connection = connection
    this.#provisionalHeaders = new RawHeaders(initializer.headers)
    request._response = this
  }

  url(): string {
    return this.#initializer.url
  }

  /** Status 0 is for `file:` URLs, as in Playwright. */
  ok(): boolean {
    const status = this.#initializer.status
    return status === 0 || (status >= 200 && status <= 299)
  }

  status(): number {
    return this.#initializer.status
  }

  statusText(): string {
    return this.#initializer.statusText
  }

  fromServiceWorker(): boolean {
    return this.#initializer.fromServiceWorker
  }

  /** The provisional headers, as Playwright's deprecated `headers()`. */
  headers(): Headers {
    return this.#provisionalHeaders.headers()
  }

  async allHeaders(): Promise<Headers> {
    return (await this.#actualHeaders('response.allHeaders')).headers()
  }

  async headersArray(): Promise<NameValue[]> {
    return (await this.#actualHeaders('response.headersArray'))
      .headersArray()
      .slice()
  }

  async headerValue(name: string): Promise<string | null> {
    return (await this.#actualHeaders('response.headerValue')).get(name)
  }

  async headerValues(name: string): Promise<string[]> {
    return (await this.#actualHeaders('response.headerValues')).getAll(name)
  }

  /**
   * Resolves once the request finished loading, as Playwright's `requestFinished` event resolves
   * it; a request that fails instead never finishes, and the page's close rejects it.
   */
  async finished(): Promise<null> {
    for (let delay = 0; ; delay = Math.min(delay * 2 || 4, 100)) {
      const state = get(this._guid, 'finished')
      if (state === 'finished') return null
      if (state === 'closed') throw targetClosedError()
      await new Promise((resolve) => setTimeout(resolve, delay))
    }
  }

  async body(): Promise<Buffer> {
    return await this.#body('response.body')
  }

  async text(): Promise<string> {
    return (await this.#body('response.text')).toString('utf8')
  }

  async json(): Promise<object> {
    return JSON.parse((await this.#body('response.json')).toString('utf8'))
  }

  request(): Request {
    return this.#request
  }

  frame(): Frame {
    return this.#request.frame()
  }

  async serverAddr(): Promise<RemoteAddr | null> {
    const { value } = await this.#connection.sendResponse(
      'response.serverAddr',
      this._guid,
      'serverAddr',
      {},
      0,
    )
    return value || null
  }

  async securityDetails(): Promise<SecurityDetails | null> {
    const { value } = await this.#connection.sendResponse(
      'response.securityDetails',
      this._guid,
      'securityDetails',
      {},
      0,
    )
    return value || null
  }

  async httpVersion(): Promise<string> {
    const { value } = await this.#connection.sendResponse(
      'response.httpVersion',
      this._guid,
      'httpVersion',
      {},
      0,
    )
    return value
  }

  toJSON(): { _type: 'Response'; _guid: string } {
    return { _type: 'Response', _guid: this._guid }
  }

  /** `Response.sizes`, which `request.sizes()` reads through this response. */
  async _sizes(apiName: ApiName): Promise<RequestSizes> {
    const { sizes } = await this.#connection.sendResponse(
      apiName,
      this._guid,
      'sizes',
      {},
      0,
    )
    return sizes
  }

  async #body(apiName: ApiName): Promise<Buffer> {
    const { binary } = await this.#connection.sendResponse(
      apiName,
      this._guid,
      'body',
      {},
      0,
    )
    return Buffer.fromBinary(binary)
  }

  #actualHeaders(apiName: ApiName): Promise<RawHeaders> {
    this.#actualHeadersPromise ??= this.#connection
      .sendResponse(apiName, this._guid, 'rawResponseHeaders', {}, 0)
      .then(({ headers }) => new RawHeaders(headers))
    return this.#actualHeadersPromise
  }
}

/**
 * The `Response` for a guid Rust handed out (`goto`'s and the navigation waits' answers), built
 * with every hop before it the first time, as {@link networkObject}. `frame`'s page owns it.
 */
export function responseFor(
  connection: Connection,
  guid: string,
  frame: Frame,
): Response {
  const built = networkObject(connection, guid, frame._page)
  if (!(built instanceof Response))
    throw new Error(
      `neo-quickjs-injected: Rust's chain for ${guid} has no such response`,
    )
  return built
}

/**
 * The `Request` or `Response` for a guid Rust handed out, built with every hop before it the
 * first time: Playwright's client has received a `__create__` for each by the time it sees one.
 */
export function networkObject(
  connection: Connection,
  guid: string,
  page: Page,
): Request | Response {
  const known = connection._objects.get(guid)
  if (known) return known
  const chain = get(guid, 'chain')
  let previous: Request | null = null
  for (const [index, created] of chain.requests.entries()) {
    let request = connection._objects.get(created.guid)
    if (!(request instanceof Request)) {
      request = new Request(
        created.guid,
        created.initializer,
        page._frameForGuid(created.initializer.frame?.guid),
        previous,
        connection,
      )
      connection._objects.set(created.guid, request)
    }
    const response = chain.responses[index]
    if (response && !connection._objects.has(response.guid))
      connection._objects.set(
        response.guid,
        new Response(response.guid, response.initializer, request, connection),
      )
    previous = request
  }
  const built = connection._objects.get(guid)
  if (!built)
    throw new Error(
      `neo-quickjs-injected: Rust's chain for ${guid} has no such object`,
    )
  return built
}

/**
 * The `Request` an event's record names: the known object, else one built from the record, its
 * earlier hops first when the script never met them. An earlier hop Rust no longer holds (past
 * its retention cap) leaves `redirectedFrom()` `null` rather than failing the event.
 */
export function requestFromRecord(
  connection: Connection,
  record: Created<RequestInitializer>,
  page: Page,
): Request {
  const known = connection._objects.get(record.guid)
  if (known instanceof Request) return known
  const from = record.initializer.redirectedFrom?.guid
  let redirectedFrom: Request | null = null
  if (from) {
    try {
      const earlier = networkObject(connection, from, page)
      if (earlier instanceof Request) redirectedFrom = earlier
    } catch {
      redirectedFrom = null
    }
  }
  const request = new Request(
    record.guid,
    record.initializer,
    page._frameForGuid(record.initializer.frame?.guid),
    redirectedFrom,
    connection,
  )
  connection._objects.set(record.guid, request)
  return request
}

/** The `Response` a `response` event's record names, for its `request` (built from the event). */
export function responseFromRecord(
  connection: Connection,
  record: Created<ResponseInitializer>,
  request: Request,
): Response {
  const known = connection._objects.get(record.guid)
  if (known instanceof Response) return known
  const response = new Response(
    record.guid,
    record.initializer,
    request,
    connection,
  )
  connection._objects.set(record.guid, response)
  return response
}

/** `URLSearchParams` entries, which QuickJS lacks, for `postDataJSON`'s form bodies. */
function parseFormBody(body: string): Record<string, string> {
  const entries: Record<string, string> = {}
  for (const pair of body.split('&')) {
    if (!pair) continue
    const at = pair.indexOf('=')
    const decode = (text: string) =>
      decodeURIComponent(text.replace(/\+/g, ' '))
    const name = decode(at < 0 ? pair : pair.slice(0, at))
    entries[name] = at < 0 ? '' : decode(pair.slice(at + 1))
  }
  return entries
}

/** Neo: `Buffer.from(text, 'utf-8')`, buffer.ts's WHATWG encoder: a lone surrogate becomes U+FFFD,
 * as Node's encoder makes it. */
export function encodeUtf8(text: string): Buffer {
  return Buffer.from(text)
}
