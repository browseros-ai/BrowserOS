/**
 * Neo: the realm's WHATWG `URL` and `URLSearchParams`, which QuickJS lacks and a Node Playwright
 * script has as globals. `quickjs.rs` installs both (`globals.ts`); a URL predicate receives this
 * `URL` (`urlFor`, Playwright's `parseURL`), and a `URL` crossing `evaluate` either way is this
 * one too (`vendor-shims.ts`), so `instanceof URL` holds wherever a script meets one.
 *
 * Rust's WHATWG parser does every parse and every setter (`parseURL` and `setURL` in
 * `runtime-bridge.ts`, over the `url` crate's implementation of the URL Standard's API); a `URL`
 * here only holds the parts Rust read and keeps its `searchParams` in step with its query, both
 * ways, as the URL Standard links them. `URLSearchParams` itself is plain JavaScript: the
 * urlencoded parser and serializer are short, and a list edit needs no trip to Rust.
 *
 * ```text
 *  new URL('/x?a=1', base) ─ parseURL ─▶ Rust ─▶ parts ─┬─ url.pathname, url.href, …
 *                                                       └─ url.searchParams (list a=1)
 *  url.search = '?b=2'     ─ setURL   ─▶ Rust ─▶ parts ─── list replaced: b=2
 *  url.searchParams.append('c', '3') ─ list a=1,c=3 ─ setURL('search', 'a=1&c=3') ─▶ url.href
 * ```
 */

import { decodeUtf8, encodeUtf8 } from './buffer'
import {
  type ParsedURL,
  parseURL,
  setURL,
  type URLPart,
} from './runtime-bridge'

/** The predicate form of Playwright's `URLMatch`. */
export type URLPredicate = (url: URL) => boolean | Promise<boolean>

/** One urlencoded name-value pair. */
type Pair = [name: string, value: string]

/** What `URL` does to its own `searchParams`, which only this module can reach. */
let linkParams: (
  params: URLSearchParams,
  update: (query: string) => void,
) => void
let resetParams: (params: URLSearchParams, pairs: readonly Pair[]) => void

/** Parts `URL.parse` already read, which the constructor it calls takes instead of parsing again.
 * Set and taken with no script code in between. */
let parsed: ParsedURL | undefined

export class URL {
  #parts: ParsedURL
  readonly #searchParams: URLSearchParams

  constructor(url: string | URL, base?: string | URL) {
    let parts = parsed
    parsed = undefined
    if (!parts) {
      const text = usv(url)
      const baseText = base === undefined ? undefined : usv(base)
      parts = parseURL(text, baseText) ?? undefined
      if (!parts) throw invalidURL(text, baseText)
    }
    this.#parts = parts
    this.#searchParams = new URLSearchParams()
    resetParams(this.#searchParams, parts.searchParams)
    linkParams(this.#searchParams, (query) => this.#set('search', query, false))
  }

  /** `new URL(url, base)`, or `null` where that throws. */
  static parse(url: string | URL, base?: string | URL): URL | null {
    const parts = parseURL(usv(url), base === undefined ? undefined : usv(base))
    if (!parts) return null
    parsed = parts
    return new URL(parts.href)
  }

  static canParse(url: string | URL, base?: string | URL): boolean {
    return (
      parseURL(usv(url), base === undefined ? undefined : usv(base)) !== null
    )
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
    return this.#searchParams
  }

  set href(value: string) {
    const text = usv(value)
    const parts = setURL(this.#parts.href, 'href', text)
    if (!parts) throw invalidURL(text)
    this.#adopt(parts, true)
  }
  set protocol(value: string) {
    this.#set('protocol', value, false)
  }
  set username(value: string) {
    this.#set('username', value, false)
  }
  set password(value: string) {
    this.#set('password', value, false)
  }
  set host(value: string) {
    this.#set('host', value, false)
  }
  set hostname(value: string) {
    this.#set('hostname', value, false)
  }
  set port(value: string) {
    this.#set('port', value, false)
  }
  set pathname(value: string) {
    this.#set('pathname', value, false)
  }
  set search(value: string) {
    this.#set('search', value, true)
  }
  set hash(value: string) {
    this.#set('hash', value, false)
  }

  toString(): string {
    return this.#parts.href
  }

  toJSON(): string {
    return this.#parts.href
  }

  get [Symbol.toStringTag](): string {
    return 'URL'
  }

  /** Runs one setter other than `href`'s, which never throws: Rust leaves the parts as they
   * were where the URL Standard ignores `value`. `refresh` re-reads `searchParams` from the new
   * query; a `searchParams` edit itself passes `false`, its list already being the query. */
  #set(part: URLPart, value: unknown, refresh: boolean): void {
    const parts = setURL(this.#parts.href, part, usv(value))
    if (parts) this.#adopt(parts, refresh)
  }

  #adopt(parts: ParsedURL, refresh: boolean): void {
    this.#parts = parts
    if (refresh) resetParams(this.#searchParams, parts.searchParams)
  }
}

/** The WHATWG `URLSearchParams`: an ordered list of name-value pairs, linked to its `URL`'s query
 * when it is one's `searchParams`. Construction follows Node's (`lib/internal/url.js`). */
export class URLSearchParams {
  #pairs: Pair[] = []
  #update: ((query: string) => void) | undefined

  static {
    linkParams = (params, update) => {
      params.#update = update
    }
    resetParams = (params, pairs) => {
      params.#pairs = pairs.map(([name, value]) => [name, value])
    }
  }

  constructor(init?: unknown) {
    if (init === undefined || init === null) return
    if (typeof init !== 'object' && typeof init !== 'function') {
      const query = usv(init)
      this.#pairs = parseQuery(query.startsWith('?') ? query.slice(1) : query)
      return
    }
    if (init instanceof URLSearchParams) {
      this.#pairs = init.#pairs.map(([name, value]) => [name, value])
      return
    }
    const iterate = (init as { [Symbol.iterator]?: unknown })[Symbol.iterator]
    if (iterate === undefined || iterate === null) {
      // A record: its own enumerable string keys, in order.
      for (const name of Object.keys(init))
        this.#pairs.push([
          usv(name),
          usv((init as Record<string, unknown>)[name]),
        ])
      return
    }
    if (typeof iterate !== 'function')
      throw new TypeError('Query pairs must be iterable')
    for (const pair of init as Iterable<unknown>) {
      const entry =
        typeof pair === 'object' &&
        pair !== null &&
        typeof (pair as { [Symbol.iterator]?: unknown })[Symbol.iterator] ===
          'function'
          ? [...(pair as Iterable<unknown>)]
          : undefined
      if (entry?.length !== 2)
        throw new TypeError(
          'Each query pair must be an iterable [name, value] tuple',
        )
      this.#pairs.push([usv(entry[0]), usv(entry[1])])
    }
  }

  get size(): number {
    return this.#pairs.length
  }

  append(name: string, value: string): void {
    this.#pairs.push([usv(name), usv(value)])
    this.#changed()
  }

  delete(name: string, value?: string): void {
    const key = usv(name)
    const only = value === undefined ? undefined : usv(value)
    this.#pairs = this.#pairs.filter(
      ([k, v]) => k !== key || (only !== undefined && v !== only),
    )
    this.#changed()
  }

  get(name: string): string | null {
    const key = usv(name)
    return this.#pairs.find(([k]) => k === key)?.[1] ?? null
  }

  getAll(name: string): string[] {
    const key = usv(name)
    return this.#pairs.filter(([k]) => k === key).map(([, value]) => value)
  }

  has(name: string, value?: string): boolean {
    const key = usv(name)
    const only = value === undefined ? undefined : usv(value)
    return this.#pairs.some(
      ([k, v]) => k === key && (only === undefined || v === only),
    )
  }

  /** Sets the first pair named `name` to `value` and drops the rest of that name, or appends. */
  set(name: string, value: string): void {
    const key = usv(name)
    const text = usv(value)
    const first = this.#pairs.findIndex(([k]) => k === key)
    if (first === -1) this.#pairs.push([key, text])
    else
      this.#pairs = this.#pairs.flatMap((pair, i): Pair[] =>
        i === first ? [[key, text]] : pair[0] === key ? [] : [pair],
      )
    this.#changed()
  }

  /** A stable sort by name, in UTF-16 code units. */
  sort(): void {
    this.#pairs.sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
    this.#changed()
  }

  forEach(
    callback: (value: string, name: string, params: URLSearchParams) => void,
    thisArg?: unknown,
  ): void {
    for (let i = 0; i < this.#pairs.length; i++) {
      const [name, value] = this.#pairs[i] as Pair
      callback.call(thisArg, value, name, this)
    }
  }

  /** Live, as the platform's: a pair added during the walk is visited. */
  *entries(): IterableIterator<[string, string]> {
    for (let i = 0; i < this.#pairs.length; i++) {
      const [name, value] = this.#pairs[i] as Pair
      yield [name, value]
    }
  }

  *keys(): IterableIterator<string> {
    for (const [name] of this.entries()) yield name
  }

  *values(): IterableIterator<string> {
    for (const [, value] of this.entries()) yield value
  }

  [Symbol.iterator](): IterableIterator<[string, string]> {
    return this.entries()
  }

  /** The WHATWG `application/x-www-form-urlencoded` serializer. */
  toString(): string {
    return this.#pairs
      .map(([name, value]) => `${formEncode(name)}=${formEncode(value)}`)
      .join('&')
  }

  get [Symbol.toStringTag](): string {
    return 'URLSearchParams'
  }

  /** The URL Standard's "update" steps: a linked `URL`'s query becomes the serialized list. */
  #changed(): void {
    this.#update?.(this.toString())
  }
}

/** `new URL(url)` for a predicate, or `null` where it would throw. */
export function urlFor(url: string): URL | null {
  return URL.parse(url)
}

/** `String.prototype.toWellFormed` (ES2024): this QuickJS has it, TypeScript's ES2023 library
 * does not declare it. */
type WellFormed = string & { toWellFormed(): string }

/** WebIDL's `USVString` conversion: the string, each lone surrogate replaced by U+FFFD. Throws a
 * `TypeError` for a symbol, as `String` does. */
function usv(value: unknown): string {
  return (String(value) as WellFormed).toWellFormed()
}

/** Node's `ERR_INVALID_URL`: a `TypeError` with the input (and base) it could not parse. */
function invalidURL(input: string, base?: string): TypeError {
  const error = new TypeError('Invalid URL')
  return Object.assign(
    error,
    { code: 'ERR_INVALID_URL', input },
    base === undefined ? {} : { base },
  )
}

/** The WHATWG `application/x-www-form-urlencoded` parser, over a query without its `?`. */
function parseQuery(query: string): Pair[] {
  const pairs: Pair[] = []
  for (const sequence of query.split('&')) {
    if (sequence === '') continue
    const at = sequence.indexOf('=')
    pairs.push(
      at === -1
        ? [formDecode(sequence), '']
        : [
            formDecode(sequence.slice(0, at)),
            formDecode(sequence.slice(at + 1)),
          ],
    )
  }
  return pairs
}

/** A `+` is a space, and each `%XX` one byte of the UTF-8 text, decoded with U+FFFD for each
 * invalid sequence; a `%` without two hex digits stays as it is. */
function formDecode(text: string): string {
  const spaced = text.replaceAll('+', ' ')
  if (!spaced.includes('%')) return spaced
  const bytes = encodeUtf8(spaced)
  const out = new Uint8Array(bytes.length)
  let length = 0
  for (let i = 0; i < bytes.length; i++) {
    const high = hexDigit(bytes[i + 1])
    const low = hexDigit(bytes[i + 2])
    if (bytes[i] === 0x25 && high >= 0 && low >= 0) {
      out[length++] = (high << 4) | low
      i += 2
    } else {
      out[length++] = bytes[i] as number
    }
  }
  return decodeUtf8(out.subarray(0, length))
}

/** An ASCII hex digit's value, or -1. */
function hexDigit(byte: number | undefined): number {
  if (byte === undefined) return -1
  if (byte >= 0x30 && byte <= 0x39) return byte - 0x30
  const lower = byte | 0x20
  return lower >= 0x61 && lower <= 0x66 ? lower - 0x57 : -1
}

/**
 * The urlencoded byte serializer: `*-._` and ASCII alphanumerics stay, a space is `+`, every
 * other UTF-8 byte is `%XX`. `encodeURIComponent` also leaves `!'()~`, so those are escaped. Every
 * pair is a `USVString`, so no lone surrogate makes it throw.
 */
function formEncode(text: string): string {
  return encodeURIComponent(text)
    .replace(/%20/g, '+')
    .replace(
      /[!'()~]/g,
      (char) => `%${char.charCodeAt(0).toString(16).toUpperCase()}`,
    )
}
