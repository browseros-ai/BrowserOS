/**
 * Neo: the `URL` and `URLSearchParams` a URL predicate receives in the realm.
 *
 * Deviation: `isomorphic/urlMatch.ts` `urlMatches`: `return match(url);` Playwright passes a URL
 * predicate a WHATWG `URL`. The realm has no `URL`, so Neo passes this read-only one, built from
 * Rust's WHATWG parser: every getter and `toString`/`toJSON` read as `URL`'s do, and
 * `searchParams` reads as `URLSearchParams`; setters and `searchParams`' mutators throw, and
 * `instanceof URL` cannot hold.
 *
 * `urlFor` is Playwright's `parseURL`: `null` where `new URL(url)` throws, which makes
 * `urlMatches` answer `false` without calling the predicate.
 */

import { type ParsedURL, parseURL } from './runtime-bridge'

/** The predicate form of Playwright's `URLMatch`. */
export type URLPredicate = (url: URL) => boolean | Promise<boolean>

/** Deviation: `isomorphic/urlMatch.ts` `urlMatches` the error a write to Neo's read-only `URL`
 * throws, where Playwright's predicate gets a writable WHATWG `URL`. */
const READ_ONLY = "Neo's URL is read-only: build a new URL string instead"

export class URL {
  readonly #parts: ParsedURL
  #searchParams: URLSearchParams | undefined

  constructor(parts: ParsedURL) {
    this.#parts = parts
  }

  get href(): string {
    return this.#parts.href
  }
  get origin(): string {
    return this.#parts.origin
  }
  get protocol(): string {
    return this.#parts.protocol
  }
  get username(): string {
    return this.#parts.username
  }
  get password(): string {
    return this.#parts.password
  }
  get host(): string {
    return this.#parts.host
  }
  get hostname(): string {
    return this.#parts.hostname
  }
  get port(): string {
    return this.#parts.port
  }
  get pathname(): string {
    return this.#parts.pathname
  }
  get search(): string {
    return this.#parts.search
  }
  get hash(): string {
    return this.#parts.hash
  }
  get searchParams(): URLSearchParams {
    this.#searchParams ??= new URLSearchParams(this.#parts.searchParams)
    return this.#searchParams
  }

  set href(_: string) {
    throw new TypeError(READ_ONLY)
  }
  set protocol(_: string) {
    throw new TypeError(READ_ONLY)
  }
  set username(_: string) {
    throw new TypeError(READ_ONLY)
  }
  set password(_: string) {
    throw new TypeError(READ_ONLY)
  }
  set host(_: string) {
    throw new TypeError(READ_ONLY)
  }
  set hostname(_: string) {
    throw new TypeError(READ_ONLY)
  }
  set port(_: string) {
    throw new TypeError(READ_ONLY)
  }
  set pathname(_: string) {
    throw new TypeError(READ_ONLY)
  }
  set search(_: string) {
    throw new TypeError(READ_ONLY)
  }
  set hash(_: string) {
    throw new TypeError(READ_ONLY)
  }

  toString(): string {
    return this.#parts.href
  }

  toJSON(): string {
    return this.#parts.href
  }
}

/** `URLSearchParams` over a URL's query pairs, read-only. */
export class URLSearchParams {
  readonly #pairs: readonly [string, string][]

  constructor(pairs: readonly [string, string][]) {
    this.#pairs = pairs.map(([name, value]) => [name, value])
  }

  get size(): number {
    return this.#pairs.length
  }

  get(name: string): string | null {
    return this.#pairs.find(([key]) => key === name)?.[1] ?? null
  }

  getAll(name: string): string[] {
    return this.#pairs.filter(([key]) => key === name).map(([, value]) => value)
  }

  has(name: string, value?: string): boolean {
    return this.#pairs.some(
      ([key, entry]) =>
        key === name && (value === undefined || entry === value),
    )
  }

  forEach(
    callback: (value: string, name: string, params: URLSearchParams) => void,
    thisArg?: unknown,
  ): void {
    for (const [name, value] of this.#pairs)
      callback.call(thisArg, value, name, this)
  }

  *entries(): IterableIterator<[string, string]> {
    for (const [name, value] of this.#pairs) yield [name, value]
  }

  *keys(): IterableIterator<string> {
    for (const [name] of this.#pairs) yield name
  }

  *values(): IterableIterator<string> {
    for (const [, value] of this.#pairs) yield value
  }

  [Symbol.iterator](): IterableIterator<[string, string]> {
    return this.entries()
  }

  /** The WHATWG `application/x-www-form-urlencoded` serializer, as `URLSearchParams` has it. */
  toString(): string {
    return this.#pairs
      .map(([name, value]) => `${formEncode(name)}=${formEncode(value)}`)
      .join('&')
  }

  append(): never {
    throw new TypeError(READ_ONLY)
  }
  delete(): never {
    throw new TypeError(READ_ONLY)
  }
  set(): never {
    throw new TypeError(READ_ONLY)
  }
  sort(): never {
    throw new TypeError(READ_ONLY)
  }
}

/** `new URL(url)` for a predicate, or `null` where it would throw. */
export function urlFor(url: string): URL | null {
  const parts = parseURL(url)
  return parts ? new URL(parts) : null
}

/**
 * The urlencoded byte serializer: `*-._` and ASCII alphanumerics stay, a space is `+`, every
 * other UTF-8 byte is `%XX`. `encodeURIComponent` also leaves `!'()~`, so those are escaped. The
 * pairs come from Rust strings, so no lone surrogate makes it throw.
 */
function formEncode(text: string): string {
  return encodeURIComponent(text)
    .replace(/%20/g, '+')
    .replace(
      /[!'()~]/g,
      (char) => `%${char.charCodeAt(0).toString(16).toUpperCase()}`,
    )
}
