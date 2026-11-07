/**
 * Playwright: `client/fetch.ts` `APIResponse`
 *
 * A response an API request context made (`api-request.ts`), whose body the context keeps in
 * Rust until `dispose()`, each `body()` asking for it; or what `route.fetch` answers, the response
 * the browser received for the routed request, its body read whole (neo-runtime `routes.rs`
 * `fetch`), which `dispose()` only lets go.
 */
import type {
  NameValue,
  RemoteAddr,
  ResourceTiming,
  SecurityDetails,
} from '@browseros/neo-runtime-spec/serve'
import { Buffer } from './buffer'
import type { NeoRouteFetchResult } from './neo-commands'
import { type Headers, RawHeaders } from './network'

/** Playwright: `client/fetch.ts` `APIResponse.body`: `throw new Error('Response has been disposed');` */
const DISPOSED = 'Response has been disposed'

/** Neo: what a timing Playwright does not know is (`timing()`'s missing value). */
const UNKNOWN_TIMING = -1

/** Neo: where a response's body is: `read` gives it, `undefined` once it is disposed. */
export interface ResponseBody {
  read(): Promise<Buffer | undefined>
  dispose(): Promise<void>
}

/** Neo: the wire's `APIResponse` fields a response reads, `fetchUid` aside. */
export interface ResponseRecord {
  url: string
  status: number
  statusText: string
  headers: NameValue[]
  securityDetails?: SecurityDetails
  serverAddr?: RemoteAddr
  timing?: ResourceTiming
  responseEndTiming?: number
}

export class APIResponse {
  readonly #record: ResponseRecord
  readonly #headers: RawHeaders
  readonly #body: ResponseBody

  constructor(record: ResponseRecord, body: ResponseBody) {
    this.#record = record
    this.#headers = new RawHeaders(record.headers)
    this.#body = body
  }

  /** `route.fetch`'s response, its body read whole. */
  static fromRouteFetch(record: NeoRouteFetchResult): APIResponse {
    let body: Buffer | undefined = Buffer.fromBinary(record.body)
    return new APIResponse(record, {
      read: async () => body,
      dispose: async () => {
        body = undefined
      },
    })
  }

  ok(): boolean {
    return this.#record.status >= 200 && this.#record.status <= 299
  }

  url(): string {
    return this.#record.url
  }

  status(): number {
    return this.#record.status
  }

  statusText(): string {
    return this.#record.statusText
  }

  headers(): Headers {
    return this.#headers.headers()
  }

  headersArray(): NameValue[] {
    return this.#headers.headersArray()
  }

  /** Playwright: `client/fetch.ts` `APIResponse.body` */
  async body(): Promise<Buffer> {
    const body = await this.#body.read()
    if (!body) throw new Error(DISPOSED)
    return body
  }

  async text(): Promise<string> {
    return (await this.body()).toString('utf8')
  }

  async json(): Promise<unknown> {
    return JSON.parse(await this.text())
  }

  async dispose(): Promise<void> {
    await this.#body.dispose()
  }

  async [Symbol.asyncDispose](): Promise<void> {
    await this.dispose()
  }

  /** Playwright: `client/fetch.ts` `APIResponse.serverAddr` */
  async serverAddr(): Promise<RemoteAddr | null> {
    return this.#record.serverAddr ?? null
  }

  /** Playwright: `client/fetch.ts` `APIResponse.securityDetails`. Deviation: Neo reads no
   * certificate (no X.509 parser in its build), so it is `null`. */
  async securityDetails(): Promise<SecurityDetails | null> {
    return this.#record.securityDetails ?? null
  }

  /** Playwright: `client/fetch.ts` `APIResponse.timing`: `responseEnd` the response's end, and
   * -1 for what Neo's client does not tell. */
  timing(): ResourceTiming & { responseEnd: number } {
    const timing = this.#record.timing ?? {
      startTime: UNKNOWN_TIMING,
      domainLookupStart: UNKNOWN_TIMING,
      domainLookupEnd: UNKNOWN_TIMING,
      connectStart: UNKNOWN_TIMING,
      secureConnectionStart: UNKNOWN_TIMING,
      connectEnd: UNKNOWN_TIMING,
      requestStart: UNKNOWN_TIMING,
      responseStart: UNKNOWN_TIMING,
    }
    return {
      ...timing,
      responseEnd: this.#record.responseEndTiming ?? UNKNOWN_TIMING,
    }
  }
}
