/**
 * Neo: the realm's bytes for Playwright's protocol `binary`, which crosses the wire as base64
 * text in both directions (neo-runtime-spec `Binary` on the Rust side). Every result that
 * carries bytes resolves a `Buffer` (`page.screenshot`, `locator.screenshot`, `page.pdf`,
 * `response.body`), and every input that carries bytes sends `toBinary(bytes)`.
 *
 * ```text
 *  Rust Binary(Vec<u8>) ── "iVBORw0KGgo…" ──▶ Buffer.fromBinary ──▶ Buffer (a Uint8Array)
 *  Rust Binary(Vec<u8>) ◀── "aGVsbG8="     ◀── toBinary(Uint8Array | Buffer)
 * ```
 *
 * `Buffer` is the part of Node's `Buffer` a script reads results with, moved unchanged in
 * behaviour from `neo-sdk/src/network.ts` (#3172). QuickJS has no `Buffer`; the realm's global
 * `Buffer` is this class, defined by `realm.rs` from `globals.ts`.
 *
 * Platform: this QuickJS (quickjs-ng in rquickjs 0.12) has `Uint8Array.fromBase64` and
 * `Uint8Array.prototype.toBase64` (probes-bin.md B3: a 10 MiB result decodes in 6 ms and
 * encodes in 2 ms, where a JavaScript loop took 1.5 s and 3.7 s), so both directions use them.
 */

/** The two base64 natives, which TypeScript's ES2023 library does not declare. */
type Base64Natives = {
  fromBase64(
    text: string,
    options?: { alphabet?: 'base64' | 'base64url' },
  ): Uint8Array
}
type Base64Bytes = Uint8Array & {
  toBase64(options?: {
    alphabet?: 'base64' | 'base64url'
    omitPadding?: boolean
  }): string
}

/** The wire's `binary` for `bytes`: standard base64 with padding, which Rust decodes. */
export function toBinary(bytes: Uint8Array): string {
  return (bytes as Base64Bytes).toBase64()
}

/**
 * The part of Node's `Buffer` a body needs: a `Uint8Array` whose `toString` decodes as Node's
 * does (UTF-8 by default), whose JSON is Node's `{ type: 'Buffer', data }`, and `equals`. It is
 * also the script's global `Buffer` (`globals.ts`), with the statics a script builds one with
 * (`from`, `alloc`, `concat`, `byteLength`, `isBuffer`, …), so a screenshot is `instanceof Buffer`.
 */
export class Buffer extends Uint8Array {
  /**
   * Node's `Buffer.from` (`lib/buffer.js`): a string's bytes in `encoding` (UTF-8 by default), a
   * view over an `ArrayBuffer`'s own memory, or a copy of a typed array, an array or array-like,
   * or a `{ type: 'Buffer', data }` JSON form.
   */
  static override from(
    value: unknown,
    encodingOrOffset?: unknown,
    length?: unknown,
  ): Buffer {
    if (typeof value === 'string')
      return Buffer.#copy(encodeString(value, encodingOrOffset))
    if (value instanceof ArrayBuffer || value instanceof SharedArrayBuffer) {
      const offset = Number(encodingOrOffset) || 0
      const size =
        length === undefined ? value.byteLength - offset : Number(length) || 0
      return new Buffer(value as ArrayBuffer, offset, size)
    }
    if (typeof value === 'object' && value !== null) {
      const primitive = (value as { valueOf?: () => unknown }).valueOf?.()
      if (
        primitive !== value &&
        primitive !== null &&
        (typeof primitive === 'string' || typeof primitive === 'object')
      )
        return Buffer.from(primitive, encodingOrOffset, length)
      const like = value as { length?: unknown; type?: unknown; data?: unknown }
      if (like.length !== undefined || ArrayBuffer.isView(value))
        return Buffer.#copy(
          typeof like.length === 'number' ? (value as ArrayLike<number>) : [],
        )
      if (like.type === 'Buffer' && Array.isArray(like.data))
        return Buffer.#copy(like.data)
    }
    throw new TypeError(
      'The first argument must be of type string or an instance of Buffer, ArrayBuffer, or Array or an Array-like Object.',
    )
  }

  /** Node's `Buffer.alloc`: `size` bytes, zeroed, or filled with a byte, a string's bytes in
   * `encoding`, or a repeated byte pattern. */
  static alloc(
    size: number,
    fill?: string | number | Uint8Array,
    encoding?: string,
  ): Buffer {
    if (typeof size !== 'number')
      throw new TypeError('The "size" argument must be of type number.')
    if (!(size >= 0))
      throw new RangeError(
        `The value of "size" is out of range. Received ${size}`,
      )
    const out = new Buffer(size)
    if (fill === undefined || fill === 0 || size === 0) return out
    if (typeof fill === 'number') return out.fill(fill & 0xff)
    const pattern =
      typeof fill === 'string' ? encodeString(fill, encoding) : fill
    if (pattern.length === 0) return out
    for (let at = 0; at < size; at += pattern.length)
      out.set(pattern.subarray(0, size - at), at)
    return out
  }

  /** Node's `Buffer.allocUnsafe`, zeroed: Neo never hands a script stale memory. */
  static allocUnsafe(size: number): Buffer {
    return Buffer.alloc(size)
  }

  /** Node's `Buffer.concat`: the bytes of every `Uint8Array` in `list`, cut or zero-padded to
   * `totalLength` when it is given. */
  static concat(list: readonly Uint8Array[], totalLength?: number): Buffer {
    if (!Array.isArray(list))
      throw new TypeError('The "list" argument must be an instance of Array.')
    for (const [i, item] of list.entries())
      if (!(item instanceof Uint8Array))
        throw new TypeError(
          `The "list[${i}]" argument must be an instance of Buffer or Uint8Array.`,
        )
    const size =
      totalLength === undefined
        ? list.reduce((sum, item) => sum + item.length, 0)
        : totalLength
    const out = Buffer.alloc(size)
    let at = 0
    for (const item of list) {
      if (at >= size) break
      const part = item.subarray(0, size - at)
      out.set(part, at)
      at += part.length
    }
    return out
  }

  /** Node's `Buffer.byteLength`: the bytes `Buffer.from(value, encoding)` would hold. */
  static byteLength(
    value: string | ArrayBufferView | ArrayBufferLike,
    encoding?: string,
  ): number {
    if (typeof value === 'string') return encodeString(value, encoding).length
    if (
      ArrayBuffer.isView(value) ||
      value instanceof ArrayBuffer ||
      value instanceof SharedArrayBuffer
    )
      return value.byteLength
    throw new TypeError(
      'The "string" argument must be of type string or an instance of Buffer or ArrayBuffer.',
    )
  }

  static isBuffer(value: unknown): value is Buffer {
    return value instanceof Buffer
  }

  static isEncoding(encoding: unknown): boolean {
    return typeof encoding === 'string' && encodingName(encoding) !== undefined
  }

  /** Node's `Buffer.compare`: -1, 0 or 1, byte by byte, a prefix first. */
  static compare(a: Uint8Array, b: Uint8Array): number {
    const length = Math.min(a.length, b.length)
    for (let i = 0; i < length; i++)
      if (a[i] !== b[i]) return (a[i] as number) < (b[i] as number) ? -1 : 1
    return a.length === b.length ? 0 : a.length < b.length ? -1 : 1
  }

  /** A new `Buffer` holding a copy of `bytes`, each value reduced to a byte as a `Uint8Array`
   * stores it. */
  static #copy(bytes: ArrayLike<number>): Buffer {
    const out = new Buffer(bytes.length)
    out.set(bytes)
    return out
  }

  /** The bytes of the wire's `binary`, a base64 string, without a copy. Rust always sends
   * standard base64 with padding; anything else is a broken bridge. */
  static fromBinary(text: string): Buffer {
    let bytes: Uint8Array
    try {
      bytes = (Uint8Array as unknown as Base64Natives).fromBase64(text)
    } catch {
      throw new Error('neo-sdk: invalid base64 from Rust')
    }
    return new Buffer(
      bytes.buffer as ArrayBuffer,
      bytes.byteOffset,
      bytes.length,
    )
  }

  /**
   * Node's `Buffer.prototype.toString` (`lib/buffer.js`): with no argument the whole buffer as
   * UTF-8. Otherwise the range is clamped and truncated first, so an empty range is `''` whatever
   * the encoding; then the encoding is read case-insensitively, an unknown one a `TypeError`.
   */
  override toString(
    ...args: [encoding?: string | null, start?: number, end?: number]
  ): string {
    if (args.length === 0) return decodeUtf8(this)
    const [encoding, start, end] = args
    const length = this.length
    let from: number
    if ((start as number) <= 0) from = 0
    else if ((start as number) >= length) return ''
    else from = Math.trunc(start as number) || 0
    const to = end === undefined || end > length ? length : Math.trunc(end) || 0
    if (to <= from) return ''
    const bytes = this.subarray(from, to)
    if (encoding === undefined) return decodeUtf8(bytes)
    switch (String(encoding).toLowerCase()) {
      case 'utf8':
      case 'utf-8':
        return decodeUtf8(bytes)
      case 'ucs2':
      case 'ucs-2':
      case 'utf16le':
      case 'utf-16le':
        return fromCodeUnits(utf16Units(bytes), (unit) => unit)
      case 'latin1':
      case 'binary':
        return fromCodeUnits(bytes, (byte) => byte)
      case 'ascii':
        return fromCodeUnits(bytes, (byte) => byte & 0x7f)
      case 'base64':
        return toBinary(bytes)
      case 'base64url':
        return (bytes as Base64Bytes).toBase64({
          alphabet: 'base64url',
          omitPadding: true,
        })
      case 'hex':
        return [...bytes]
          .map((byte) => byte.toString(16).padStart(2, '0'))
          .join('')
      default:
        throw new TypeError(`Unknown encoding: ${encoding}`)
    }
  }

  toJSON(): { type: 'Buffer'; data: number[] } {
    return { type: 'Buffer', data: [...this] }
  }

  equals(other: Uint8Array): boolean {
    return (
      this.length === other.length && this.every((byte, i) => byte === other[i])
    )
  }
}

/** A UTF-8 lead byte's sequence: continuation bytes still needed, its payload bits, and the
 * range its first continuation byte must fall in (the WHATWG decoder's bounds). */
export type Lead = {
  needed: number
  codePoint: number
  lower: number
  upper: number
}

export function utf8Lead(byte: number): Lead | null {
  if (byte >= 0xc2 && byte <= 0xdf)
    return { needed: 1, codePoint: byte & 0x1f, lower: 0x80, upper: 0xbf }
  if (byte >= 0xe0 && byte <= 0xef)
    return {
      needed: 2,
      codePoint: byte & 0xf,
      lower: byte === 0xe0 ? 0xa0 : 0x80,
      upper: byte === 0xed ? 0x9f : 0xbf,
    }
  if (byte >= 0xf0 && byte <= 0xf4)
    return {
      needed: 3,
      codePoint: byte & 0x7,
      lower: byte === 0xf0 ? 0x90 : 0x80,
      upper: byte === 0xf4 ? 0x8f : 0xbf,
    }
  return null
}

/**
 * The WHATWG UTF-8 decoder, which Node's `buffer.toString('utf8')` follows: each maximal
 * invalid subsequence becomes one U+FFFD. QuickJS has no `TextDecoder`.
 */
export function decodeUtf8(bytes: Uint8Array): string {
  const units = new CodeUnits()
  let lead: Lead | null = null
  for (let i = 0; i < bytes.length; i++) {
    const byte = bytes[i] as number
    if (!lead) {
      lead = byte <= 0x7f ? null : utf8Lead(byte)
      if (byte <= 0x7f) units.push(byte)
      else if (!lead) units.push(0xfffd)
      continue
    }
    if (byte < lead.lower || byte > lead.upper) {
      // The sequence ends here; this byte starts over.
      lead = null
      units.push(0xfffd)
      i--
      continue
    }
    lead.codePoint = (lead.codePoint << 6) | (byte & 0x3f)
    lead.lower = 0x80
    lead.upper = 0xbf
    lead.needed--
    if (lead.needed === 0) {
      units.push(lead.codePoint)
      lead = null
    }
  }
  if (lead) units.push(0xfffd)
  return units.text()
}

/** UTF-16 code units collected in chunks, so a long body never spreads one huge array. */
class CodeUnits {
  #units: number[] = []
  #out = ''

  push(codePoint: number): void {
    if (codePoint > 0xffff) {
      const offset = codePoint - 0x10000
      this.#units.push(0xd800 + (offset >> 10), 0xdc00 + (offset & 0x3ff))
    } else {
      this.#units.push(codePoint)
    }
    if (this.#units.length >= 4096) this.#flush()
  }

  text(): string {
    this.#flush()
    return this.#out
  }

  #flush(): void {
    this.#out += String.fromCharCode(...this.#units)
    this.#units = []
  }
}

/** Little-endian 16-bit code units; an odd last byte is dropped, as Node drops it. */
function utf16Units(bytes: Uint8Array): Uint16Array {
  const units = new Uint16Array(bytes.length >> 1)
  for (let i = 0; i < units.length; i++)
    units[i] = (bytes[2 * i] as number) | ((bytes[2 * i + 1] as number) << 8)
  return units
}

function fromCodeUnits(
  bytes: Uint8Array | Uint16Array,
  unit: (byte: number) => number,
): string {
  let out = ''
  for (let i = 0; i < bytes.length; i += 4096)
    out += String.fromCharCode(...[...bytes.subarray(i, i + 4096)].map(unit))
  return out
}

/**
 * The WHATWG UTF-8 encoder, which Node's `Buffer.from(string)` and `TextEncoder` follow: a lone
 * surrogate becomes U+FFFD. One pass sizes the result, so a long string never needs three bytes
 * per code unit of scratch.
 */
export function encodeUtf8(text: string): Uint8Array {
  let length = 0
  for (let i = 0; i < text.length; i++) {
    const unit = text.charCodeAt(i)
    if (unit < 0x80) length += 1
    else if (unit < 0x800) length += 2
    else if (isPair(text, i)) {
      length += 4
      i++
    } else length += 3
  }
  const out = new Uint8Array(length)
  let at = 0
  for (let i = 0; i < text.length; i++) {
    let code = text.charCodeAt(i)
    if (isPair(text, i)) {
      code = 0x10000 + ((code - 0xd800) << 10) + (text.charCodeAt(++i) - 0xdc00)
    } else if (code >= 0xd800 && code <= 0xdfff) code = 0xfffd
    if (code < 0x80) out[at++] = code
    else if (code < 0x800) {
      out[at++] = 0xc0 | (code >> 6)
      out[at++] = 0x80 | (code & 0x3f)
    } else if (code < 0x10000) {
      out[at++] = 0xe0 | (code >> 12)
      out[at++] = 0x80 | ((code >> 6) & 0x3f)
      out[at++] = 0x80 | (code & 0x3f)
    } else {
      out[at++] = 0xf0 | (code >> 18)
      out[at++] = 0x80 | ((code >> 12) & 0x3f)
      out[at++] = 0x80 | ((code >> 6) & 0x3f)
      out[at++] = 0x80 | (code & 0x3f)
    }
  }
  return out
}

/** Whether `text[i]` starts a surrogate pair. */
function isPair(text: string, i: number): boolean {
  const unit = text.charCodeAt(i)
  const next = text.charCodeAt(i + 1)
  return unit >= 0xd800 && unit <= 0xdbff && next >= 0xdc00 && next <= 0xdfff
}

/** Node's name for one of its buffer encodings, read case-insensitively, or `undefined`. */
function encodingName(encoding: string): string | undefined {
  switch (encoding.toLowerCase()) {
    case 'utf8':
    case 'utf-8':
      return 'utf8'
    case 'ucs2':
    case 'ucs-2':
    case 'utf16le':
    case 'utf-16le':
      return 'utf16le'
    case 'latin1':
    case 'binary':
      return 'latin1'
    case 'ascii':
    case 'base64':
    case 'base64url':
    case 'hex':
      return encoding.toLowerCase()
    default:
      return undefined
  }
}

/**
 * A string's bytes in one of Node's encodings (`lib/buffer.js` `fromString`): UTF-8 when
 * `encoding` is not a non-empty string, a `TypeError` for an unknown one. As Node's, `latin1`
 * and `ascii` keep each code unit's low byte, base64 accepts either alphabet and skips what is
 * not a digit, and hex stops at the first pair that is not two hex digits.
 */
function encodeString(text: string, encoding: unknown): Uint8Array {
  const name =
    typeof encoding === 'string' && encoding !== ''
      ? encodingName(encoding)
      : 'utf8'
  switch (name) {
    case 'utf8':
      return encodeUtf8(text)
    case 'utf16le': {
      const out = new Uint8Array(text.length * 2)
      for (let i = 0; i < text.length; i++) {
        const unit = text.charCodeAt(i)
        out[2 * i] = unit & 0xff
        out[2 * i + 1] = unit >> 8
      }
      return out
    }
    case 'latin1':
    case 'ascii': {
      const out = new Uint8Array(text.length)
      for (let i = 0; i < text.length; i++) out[i] = text.charCodeAt(i) & 0xff
      return out
    }
    case 'base64':
    case 'base64url': {
      let digits = text
        .replace(/=.*$/s, '')
        .replace(/-/g, '+')
        .replace(/_/g, '/')
        .replace(/[^A-Za-z0-9+/]/g, '')
      // A lone last digit carries no whole byte; Node drops it.
      if (digits.length % 4 === 1) digits = digits.slice(0, -1)
      return (Uint8Array as unknown as Base64Natives).fromBase64(digits)
    }
    case 'hex': {
      const out = new Uint8Array(text.length >> 1)
      let length = 0
      for (; length < out.length; length++) {
        const pair = text.slice(2 * length, 2 * length + 2)
        if (!/^[0-9a-fA-F]{2}$/.test(pair)) break
        out[length] = Number.parseInt(pair, 16)
      }
      return out.subarray(0, length)
    }
    default:
      throw new TypeError(`Unknown encoding: ${encoding}`)
  }
}
