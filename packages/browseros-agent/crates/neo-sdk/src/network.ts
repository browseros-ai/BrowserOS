/**
 * Playwright's client `Request`, `Response` and `RawHeaders` (`client/network.ts` at 26137face),
 * for the objects behind `page.goto`'s answer: the navigation's request, one per redirect, and
 * each one's response. Rust records them from the tab's Network events (neo-runtime
 * `network.rs`) and mints their guids; `responseFor` builds the whole chain from Rust's `chain`
 * getter, once per guid, through the connection's object map, so a guid is always one object.
 *
 * Playwright's client keeps some of this state current from events (`requestFinished`,
 * `requestFailed`): `timing().responseEnd`, `failure()` and `finished()`. Neo's bridge has no
 * events, so those read Rust's record live, through getters.
 *
 * What differs from Playwright:
 * - `body()` resolves a `Uint8Array` subclass named `Buffer` with Node's `toString(encoding)`,
 *   `toJSON()` and `equals()` (`buffer.ts`); QuickJS has no `Buffer`, and the SDK defines no
 *   globals.
 * - `finished()` polls Rust's record (every few to 100 ms) instead of waiting for an event.
 * - `serviceWorker()` is always `null`: Neo answers only main-frame navigations.
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
    return this.#initializer.url
  }

  resourceType(): string {
    return this.#initializer.resourceType
  }

  method(): string {
    return this.#initializer.method
  }

  postData(): string | null {
    return this.postDataBuffer()?.toString('utf-8') ?? null
  }

  postDataBuffer(): Buffer | null {
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

  /** The provisional headers, as Playwright's deprecated `headers()`. */
  headers(): Headers {
    return this.#provisionalHeaders.headers()
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

  existingResponse(): Response | null {
    return this._response
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

  redirectedTo(): Request | null {
    return this._redirectedTo
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
      ? responseFor(this.#connection, response.guid, this.#frame)
      : null
  }

  #actualHeaders(apiName: ApiName): Promise<RawHeaders> {
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
 * The `Response` for a guid Rust handed out, built with every hop before it the first time:
 * Playwright's client has received a `__create__` for each by the time `goto` replies.
 */
export function responseFor(
  connection: Connection,
  guid: string,
  frame: Frame,
): Response {
  const known = connection._objects.get(guid)
  if (known instanceof Response) return known
  const chain = get(guid, 'chain')
  let previous: Request | null = null
  for (const [index, created] of chain.requests.entries()) {
    let request = connection._objects.get(created.guid)
    if (!(request instanceof Request)) {
      request = new Request(
        created.guid,
        created.initializer,
        frame,
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
  if (!(built instanceof Response))
    throw new Error(`neo-sdk: Rust's chain for ${guid} has no such response`)
  return built
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
