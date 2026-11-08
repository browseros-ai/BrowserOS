/**
 * Neo: Playwright's header helpers the SDK's `setExtraHTTPHeaders` needs, on the context and on a
 * page: the client's check that every value is a string, and the object-to-list conversion its
 * protocol calls send.
 */

import type { NameValue } from '@browseros/neo-runtime-spec/serve'

/** Playwright's `Headers`: a header name to its value. */
export type Headers = Record<string, string>

/** Playwright: `client/network.ts` `validateHeaders`: `if (!Object.is(value, undefined) && !isString(value))` */
export function validateHeaders(headers: Headers): void {
  for (const key of Object.keys(headers)) {
    const value: unknown = headers[key]
    if (!Object.is(value, undefined) && typeof value !== 'string')
      throw new Error(
        `Expected value of header "${key}" to be String, but "${typeof value}" is found.`,
      )
  }
}

/** Playwright: `isomorphic/headers.ts` `headersObjectToArray`: `result.push({ name, value: values });`
 * Each header once, in the object's order; an `undefined` value is left out. */
export function headersObjectToArray(headers: Headers): NameValue[] {
  const result: NameValue[] = []
  for (const name in headers) {
    const value = headers[name]
    if (value === undefined) continue
    result.push({ name, value })
  }
  return result
}
