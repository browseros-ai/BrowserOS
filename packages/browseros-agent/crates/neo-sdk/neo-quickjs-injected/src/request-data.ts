/**
 * Playwright: `client/fetch.ts` `APIRequestContext._innerFetch`
 * Playwright: `server/fetch.ts` `serializePostData`
 *
 * A request's `data`, typed as Playwright's client types it (JSON text or bytes), and the body
 * and content type its server sends for each kind: what `route.fetch` sends (`route.ts`).
 */

import type { NameValue } from '@browseros/neo-playwright-protocol/serve'
import { encodeUtf8 } from './network'

/** Neo: `data` as Playwright's client sends it, `jsonData` or `postData`. */
export type TypedData = { json?: string; bytes?: Uint8Array }

/** Neo: the body a request sends, and the content type it gets unless it has one. */
export type SerializedData = { body: Uint8Array; contentType: string }

/** Playwright: `client/fetch.ts` `isJsonContentType`: `return value.split(';')[0].trim().toLowerCase() === 'application/json';` */
export function isJsonContentType(headers?: NameValue[]): boolean {
  if (!headers) return false
  for (const { name, value } of headers) {
    if (name.toLocaleLowerCase() === 'content-type')
      return value.split(';')[0].trim().toLowerCase() === 'application/json'
  }
  return false
}

/** Playwright: `client/fetch.ts` `isJsonParsable`: `if (e instanceof SyntaxError)` */
export function isJsonParsable(value: unknown): boolean {
  if (typeof value !== 'string') return false
  try {
    JSON.parse(value)
    return true
  } catch (error) {
    if (error instanceof SyntaxError) return false
    throw error
  }
}

/**
 * Playwright: `client/fetch.ts` `APIRequestContext._innerFetch`: `jsonData = isJsonParsable(options.data) ? options.data : JSON.stringify(options.data);`
 *
 * `data` under the request's `headers`: a string is JSON text under a JSON content type and
 * bytes otherwise; an object, number, boolean or `null` is JSON; `undefined` is no data.
 *
 * Deviation: `client/fetch.ts` `APIRequestContext._innerFetch`: `} else if (Buffer.isBuffer(options.data)) {`
 * Any `Uint8Array` is bytes, as Neo's binary seam takes it (`buffer.ts`); Playwright's client
 * sends only a Node `Buffer` so, and JSON-encodes another `Uint8Array`.
 */
export function typeData(data: unknown, headers?: NameValue[]): TypedData {
  if (data === undefined) return {}
  if (typeof data === 'string') {
    if (isJsonContentType(headers))
      return { json: isJsonParsable(data) ? data : JSON.stringify(data) }
    return { bytes: encodeUtf8(data) }
  }
  if (data instanceof Uint8Array) return { bytes: data }
  if (
    typeof data === 'object' ||
    typeof data === 'number' ||
    typeof data === 'boolean'
  )
    return { json: JSON.stringify(data) }
  throw new Error(`Unexpected 'data' type`)
}

/**
 * Playwright: `server/fetch.ts` `serializePostData`: `setHeader(headers, 'content-type', 'application/octet-stream', true);`
 *
 * The body Playwright's server sends for typed data: JSON text as `application/json`, bytes as
 * `application/octet-stream`, each unless the request has a content type already.
 */
export function serializePostData(data: TypedData): SerializedData | undefined {
  if (data.json !== undefined)
    return { body: encodeUtf8(data.json), contentType: 'application/json' }
  if (data.bytes !== undefined)
    return { body: data.bytes, contentType: 'application/octet-stream' }
  return undefined
}
