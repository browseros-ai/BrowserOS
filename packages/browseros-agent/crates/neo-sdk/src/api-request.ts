/**
 * Playwright: `client/fetch.ts` `APIRequest`
 * Playwright: `client/fetch.ts` `APIRequestContext`
 *
 * Playwright's API request contexts: `context.request` (which `page.request` is), the browser
 * context's, and `playwright.request.newContext()`'s, each with its own cookie jar. Neo sends
 * their requests from Rust as Playwright's server does from Node (neo-runtime
 * `request_contexts.rs`, `api_request.rs`); a response's body stays there until it is disposed.
 */
import type {
  APIRequestContextCommands,
  FormField,
  NameValue,
  NetworkCookie,
  SetNetworkCookie,
} from '@browseros/neo-runtime-spec/serve'
import { APIResponse } from './api-response'
import {
  CLEAR_ALL_COOKIES_REFUSED,
  type ClearCookieOptions,
  clearCookieFilter,
  clearCookieMatches,
  hasClearCookieFilter,
} from './browser-context'
import { Buffer, toBinary } from './buffer'
import type { ApiName, Connection } from './connection'
import { isTargetClosedError, targetClosedError } from './errors'
import { encodeUtf8, type Headers, type Request } from './network'
import { type TimeoutInput, TimeoutSettings } from './timeout-settings'
import { wrapApiCall } from './waiter'

/** Playwright: `client/fetch.ts` `APIRequestContext._innerFetch`: `assert((options.data === undefined ? 0 : 1) + (options.form === undefined ? 0 : 1) + (options.multipart === undefined ? 0 : 1) <= 1, `Only one of 'data', 'form' or 'multipart' can be specified`);` */
const ONLY_ONE_BODY =
  "Only one of 'data', 'form' or 'multipart' can be specified"

/** Playwright: `client/fetch.ts` `APIRequestContext._innerFetch`: `assert(options.maxRedirects === undefined || options.maxRedirects >= 0, `'maxRedirects' must be greater than or equal to '0'`);` */
const MAX_REDIRECTS_NEGATIVE =
  "'maxRedirects' must be greater than or equal to '0'"

/** Playwright: `client/fetch.ts` `APIRequestContext._innerFetch`: `assert(options.maxRetries === undefined || options.maxRetries >= 0, `'maxRetries' must be greater than or equal to '0'`);` */
const MAX_RETRIES_NEGATIVE = "'maxRetries' must be greater than or equal to '0'"

/** Playwright: `client/fetch.ts` `APIRequestContext._innerFetch`: `assert(options.request || typeof options.url === 'string', 'First argument must be either URL string or Request');` */
const URL_OR_REQUEST = 'First argument must be either URL string or Request'

/** Playwright: `client/fetch.ts` `APIRequestContext._innerFetch`: `throw new Error(`Unexpected 'data' type`);` */
const UNEXPECTED_DATA = "Unexpected 'data' type"

/** Deviation: `client/fetch.ts` `APIRequest.newContext`: `const storageState = typeof options.storageState === 'string' ?`
 * Playwright reads a storage state file. A run has no file system, so Neo takes the state
 * object only. */
const STATE_FILE =
  'storageState: Neo takes a storage state object, not a file path, in a run'

/** Deviation: `client/fetch.ts` `APIRequestContext.storageState`: `if (options.path) {`
 * Playwright writes the state to `path`. A run has no file system: Neo returns it only. */
const STATE_PATH =
  'path: Neo returns the storage state, and writes no file, in a run'

/** Deviation: `client/fetch.ts` `toFormField`: `if (isFilePayload(value)) {`
 * Playwright reads a `ReadStream`'s file. A run has no file system: a file goes as
 * `{ name, mimeType, buffer }`. */
const MULTIPART_STREAM =
  'multipart: Neo takes a file as { name, mimeType, buffer }; a run has no file system to stream from'

/** Neo: the timeout Playwright's contexts start with (`client/timeoutSettings.ts`). */
const DEFAULT_TIMEOUT = 30_000

/** Playwright's `FetchOptions` (`client/fetch.ts`). */
export interface FetchOptions extends TimeoutInput {
  params?: Record<string, string | number | boolean> | string
  method?: string
  headers?: Headers
  data?: unknown
  form?: Record<string, string | number | boolean>
  multipart?: Record<string, string | number | boolean | FilePayload>
  failOnStatusCode?: boolean
  ignoreHTTPSErrors?: boolean
  maxRedirects?: number
  maxRetries?: number
}

/** Playwright's `FilePayload`. */
export interface FilePayload {
  name: string
  mimeType: string
  buffer: Uint8Array
}

/** Playwright's `NewContextOptions` for `APIRequest.newContext` (`client/fetch.ts`). */
export interface NewContextOptions {
  baseURL?: string
  userAgent?: string
  extraHTTPHeaders?: Headers
  httpCredentials?: {
    username: string
    password: string
    origin?: string
    send?: 'unauthorized' | 'always'
  }
  ignoreHTTPSErrors?: boolean
  proxy?: {
    server: string
    bypass?: string
    username?: string
    password?: string
  }
  timeout?: number
  storageState?: string | { cookies?: NetworkCookie[]; origins?: unknown[] }
  failOnStatusCode?: boolean
  maxRedirects?: number
}

/** Playwright: `client/fetch.ts` `APIRequest` */
export class APIRequest {
  readonly #guid: string
  readonly #connection: Connection

  /** `guid`: the script's `playwright` object, which `newRequest` goes to. */
  constructor(guid: string, connection: Connection) {
    this.#guid = guid
    this.#connection = connection
  }

  /** Playwright: `client/fetch.ts` `APIRequest.newContext` */
  async newContext(
    options: NewContextOptions = {},
  ): Promise<APIRequestContext> {
    // A refusal of the client's own carries the method's name; the call's errors come back
    // named from the connection.
    await wrapApiCall('apiRequest.newContext', async () => {
      if (typeof options.storageState === 'string') throw new Error(STATE_FILE)
    })
    const credentials = options.httpCredentials
    const { request } = await this.#connection.sendPlaywright(
      'apiRequest.newContext',
      this.#guid,
      'newRequest',
      {
        baseURL: options.baseURL,
        userAgent: options.userAgent,
        ignoreHTTPSErrors: options.ignoreHTTPSErrors,
        extraHTTPHeaders: options.extraHTTPHeaders
          ? headersArray(options.extraHTTPHeaders)
          : undefined,
        failOnStatusCode: options.failOnStatusCode,
        maxRedirects: options.maxRedirects,
        // Playwright: `client/browserContext.ts` `toHttpCredentialsProtocol`
        httpCredentials: credentials ? [credentials] : undefined,
        proxy: options.proxy,
        storageState: options.storageState as never,
      },
      0,
    )
    const context = new APIRequestContext(
      request.guid,
      this.#connection,
      new TimeoutSettings(),
      true,
    )
    context._timeoutSettings.setDefaultTimeout(
      options.timeout ?? DEFAULT_TIMEOUT,
    )
    return context
  }
}

/** Playwright: `client/fetch.ts` `APIRequestContext` */
export class APIRequestContext {
  readonly _timeoutSettings: TimeoutSettings
  readonly #guid: string
  readonly #connection: Connection
  /** A `newContext()`'s, with its own jar, which `clearCookies()` may empty. */
  readonly #ownJar: boolean
  #closeReason: string | undefined

  /** `timeoutSettings`: the browser context's, whose `setDefaultTimeout` this follows
   * (`context.request`); a `newContext()`'s own otherwise. */
  constructor(
    guid: string,
    connection: Connection,
    timeoutSettings: TimeoutSettings,
    ownJar: boolean,
  ) {
    this.#guid = guid
    this.#connection = connection
    this._timeoutSettings = timeoutSettings
    this.#ownJar = ownJar
  }

  async [Symbol.asyncDispose](): Promise<void> {
    await this.dispose()
  }

  /** Playwright: `client/fetch.ts` `APIRequestContext.dispose` */
  async dispose(options: { reason?: string } = {}): Promise<void> {
    this.#closeReason = options.reason
    try {
      await this.#connection.sendRequestContext(
        'apiRequestContext.dispose',
        this.#guid,
        'dispose',
        { reason: options.reason },
        0,
      )
    } catch (error) {
      if (isTargetClosedError(error)) return
      throw error
    }
  }

  async delete(url: string, options: FetchOptions = {}): Promise<APIResponse> {
    return await this.#method(
      'apiRequestContext.delete',
      'DELETE',
      url,
      options,
    )
  }

  async head(url: string, options: FetchOptions = {}): Promise<APIResponse> {
    return await this.#method('apiRequestContext.head', 'HEAD', url, options)
  }

  async get(url: string, options: FetchOptions = {}): Promise<APIResponse> {
    return await this.#method('apiRequestContext.get', 'GET', url, options)
  }

  async patch(url: string, options: FetchOptions = {}): Promise<APIResponse> {
    return await this.#method('apiRequestContext.patch', 'PATCH', url, options)
  }

  async post(url: string, options: FetchOptions = {}): Promise<APIResponse> {
    return await this.#method('apiRequestContext.post', 'POST', url, options)
  }

  async put(url: string, options: FetchOptions = {}): Promise<APIResponse> {
    return await this.#method('apiRequestContext.put', 'PUT', url, options)
  }

  /** Playwright: `client/fetch.ts` `APIRequestContext.fetch` */
  async fetch(
    urlOrRequest: string | Request,
    options: FetchOptions = {},
  ): Promise<APIResponse> {
    const url = typeof urlOrRequest === 'string' ? urlOrRequest : undefined
    const request = typeof urlOrRequest === 'string' ? undefined : urlOrRequest
    return await this._innerFetch('apiRequestContext.fetch', {
      url,
      request,
      ...options,
    })
  }

  /** Playwright: `client/fetch.ts` `APIRequestContext.get`: `return await this.fetch(url, { ...options, method: 'GET' });` */
  async #method(
    apiName: ApiName,
    method: string,
    url: string,
    options: FetchOptions,
  ): Promise<APIResponse> {
    return await this._innerFetch(apiName, { url, ...options, method })
  }

  /** Playwright: `client/fetch.ts` `APIRequestContext._innerFetch` */
  async _innerFetch(
    apiName: ApiName,
    options: FetchOptions & { url?: string; request?: Request },
  ): Promise<APIResponse> {
    // The client's checks carry the method's name; the call's errors come back named.
    const params = await wrapApiCall(apiName, async () =>
      this.#fetchParams(options),
    )
    const { response } = await this.#connection.sendRequestContext(
      apiName,
      this.#guid,
      'fetch',
      params,
      this._timeoutSettings.timeout(options, apiName),
    )
    return new APIResponse(response, {
      read: async () => {
        const { binary } = await this.#connection.sendRequestContext(
          'apiResponse.body',
          this.#guid,
          'fetchResponseBody',
          { fetchUid: response.fetchUid },
          0,
        )
        return binary === undefined ? undefined : Buffer.fromBinary(binary)
      },
      dispose: async () => {
        await this.#connection.sendRequestContext(
          'apiResponse.dispose',
          this.#guid,
          'disposeAPIResponse',
          { fetchUid: response.fetchUid },
          0,
        )
      },
    })
  }

  /** Playwright: `client/fetch.ts` `APIRequestContext._innerFetch`: the fetch's params, after the
   * client's checks. */
  #fetchParams(
    options: FetchOptions & { url?: string; request?: Request },
  ): APIRequestContextCommands['fetch']['params'] {
    if (this.#closeReason) throw targetClosedError(this.#closeReason)
    if (!options.request && typeof options.url !== 'string')
      throw new Error(URL_OR_REQUEST)
    const bodies =
      (options.data === undefined ? 0 : 1) +
      (options.form === undefined ? 0 : 1) +
      (options.multipart === undefined ? 0 : 1)
    if (bodies > 1) throw new Error(ONLY_ONE_BODY)
    if (options.maxRedirects !== undefined && options.maxRedirects < 0)
      throw new Error(MAX_REDIRECTS_NEGATIVE)
    if (options.maxRetries !== undefined && options.maxRetries < 0)
      throw new Error(MAX_RETRIES_NEGATIVE)
    const url = options.url ?? options.request?.url() ?? ''
    const method = options.method || options.request?.method()
    const encodedParams =
      typeof options.params === 'string' ? options.params : undefined
    const headersObject = options.headers || options.request?.headers()
    const headers = headersObject ? headersArray(headersObject) : undefined
    let jsonData: string | undefined
    let formData: NameValue[] | undefined
    let multipartData: FormField[] | undefined
    let postData: Uint8Array | undefined
    if (options.data !== undefined) {
      if (typeof options.data === 'string') {
        if (isJsonContentType(headers))
          jsonData = isJsonParsable(options.data)
            ? options.data
            : JSON.stringify(options.data)
        else postData = encodeUtf8(options.data)
      } else if (options.data instanceof Uint8Array) {
        postData = options.data
      } else if (
        typeof options.data === 'object' ||
        typeof options.data === 'number' ||
        typeof options.data === 'boolean'
      ) {
        jsonData = JSON.stringify(options.data)
      } else {
        throw new Error(UNEXPECTED_DATA)
      }
    } else if (options.form) {
      formData = objectToArray(options.form)
    } else if (options.multipart) {
      multipartData = Object.entries(options.multipart).map(([name, value]) =>
        toFormField(name, value),
      )
    }
    if (
      postData === undefined &&
      jsonData === undefined &&
      formData === undefined &&
      multipartData === undefined
    )
      postData = options.request?.postDataBuffer() ?? undefined
    return {
      url,
      params:
        typeof options.params === 'object'
          ? objectToArray(options.params)
          : undefined,
      encodedParams,
      method,
      headers,
      postData: postData ? toBinary(postData) : undefined,
      jsonData,
      formData,
      multipartData,
      failOnStatusCode: options.failOnStatusCode,
      ignoreHTTPSErrors: options.ignoreHTTPSErrors,
      maxRedirects: options.maxRedirects,
      maxRetries: options.maxRetries,
    }
  }

  /** Playwright: `client/fetch.ts` `APIRequestContext.storageState` */
  async storageState(
    options: { path?: string; indexedDB?: boolean; opfs?: boolean } = {},
  ): Promise<unknown> {
    await wrapApiCall('apiRequestContext.storageState', async () => {
      if (options.path) throw new Error(STATE_PATH)
    })
    return await this.#connection.sendRequestContext(
      'apiRequestContext.storageState',
      this.#guid,
      'storageState',
      { indexedDB: options.indexedDB, opfs: options.opfs },
      0,
    )
  }

  /** Playwright: `client/fetch.ts` `APIRequestContext.cookies` */
  async cookies(urls?: string | string[]): Promise<NetworkCookie[]> {
    const list = urls === undefined ? [] : Array.isArray(urls) ? urls : [urls]
    const { cookies } = await this.#connection.sendRequestContext(
      'apiRequestContext.cookies',
      this.#guid,
      'cookies',
      { urls: list },
      0,
    )
    return cookies
  }

  /** Playwright: `client/fetch.ts` `APIRequestContext.addCookies` */
  async addCookies(cookies: SetNetworkCookie[]): Promise<void> {
    await this.#connection.sendRequestContext(
      'apiRequestContext.addCookies',
      this.#guid,
      'addCookies',
      { cookies },
      0,
    )
  }

  /**
   * Playwright: `client/fetch.ts` `APIRequestContext.clearCookies`
   *
   * As the browser context's (`browser-context.ts`): the matching cookies expire through
   * `addCookies`. The browser context's request context refuses to clear every cookie of the
   * user's browser, as its context does.
   */
  async clearCookies(options: ClearCookieOptions = {}): Promise<void> {
    const filter = clearCookieFilter(options)
    await wrapApiCall('apiRequestContext.clearCookies', async () => {
      if (!this.#ownJar && !hasClearCookieFilter(filter))
        throw new Error(CLEAR_ALL_COOKIES_REFUSED)
    })
    const { cookies } = await this.#connection.sendRequestContext(
      'apiRequestContext.clearCookies',
      this.#guid,
      'cookies',
      { urls: [] },
      0,
    )
    const expired = cookies
      .filter((cookie) => clearCookieMatches(cookie, filter))
      .map((cookie) => ({ ...cookie, expires: 0 }))
    if (!expired.length) return
    await this.#connection.sendRequestContext(
      'apiRequestContext.clearCookies',
      this.#guid,
      'addCookies',
      { cookies: expired },
      0,
    )
  }
}

/** Playwright: `client/fetch.ts` `objectToArray` */
function objectToArray(
  map: Record<string, string | number | boolean>,
): NameValue[] {
  return Object.entries(map).map(([name, value]) => ({
    name,
    value: String(value),
  }))
}

/** Playwright: `utils/isomorphic/headers.ts` `headersObjectToArray` */
function headersArray(headers: Headers): NameValue[] {
  const result: NameValue[] = []
  for (const name in headers) {
    const value = headers[name]
    if (value !== undefined) result.push({ name, value: String(value) })
  }
  return result
}

/** Playwright: `client/fetch.ts` `isJsonContentType` */
function isJsonContentType(headers?: NameValue[]): boolean {
  if (!headers) return false
  for (const { name, value } of headers) {
    if (name.toLocaleLowerCase() === 'content-type')
      return value === 'application/json'
  }
  return false
}

/** Playwright: `client/fetch.ts` `isJsonParsable` */
function isJsonParsable(value: unknown): boolean {
  if (typeof value !== 'string') return false
  try {
    JSON.parse(value)
    return true
  } catch (error) {
    if (error instanceof SyntaxError) return false
    throw error
  }
}

/** Playwright: `client/fetch.ts` `toFormField` */
function toFormField(
  name: string,
  value: string | number | boolean | FilePayload,
): FormField {
  if (
    typeof value === 'object' &&
    value !== null &&
    'buffer' in value &&
    value.buffer instanceof Uint8Array
  ) {
    return {
      name,
      file: {
        name: value.name,
        mimeType: value.mimeType,
        buffer: toBinary(value.buffer),
      },
    }
  }
  if (typeof value === 'object' && value !== null)
    throw new Error(MULTIPART_STREAM)
  return { name, value: String(value) }
}
