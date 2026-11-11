/**
 * Node's `Buffer`, as far as Playwright scripts use it, for QuickJS, which has none. Upstream's
 * examples build `setInputFiles` payloads with `Buffer.from('…')`, and upstream's server turns a
 * payload's bytes into base64 with `buffer.toString('base64')`, which only a Buffer answers
 * with base64. Screenshots and PDFs resolve to one, as upstream's do (the bytes brand revives
 * as one, index.ts).
 *
 * It is a Uint8Array subclass, so the bootstrap's codec still sends one as its bytes brand and
 * every Uint8Array method works on it. Its encodings are Node's (encodings.ts). It has no
 * `toJSON`, unlike Node's: the bootstrap turns a run's returned bytes into the bytes brand only
 * while `JSON.stringify` has no `toJSON` to call first.
 */
import { decode, type Encoding, encode, encodingNamed } from './encodings'
import { invalidValue, nodeError, outOfRange, received } from './node-errors'

/** Node's `buffer.constants.MAX_LENGTH` on 64-bit, the bound of a fill's offset. */
const MAX_LENGTH = Number.MAX_SAFE_INTEGER

/** An encoding Node knows, or Node's `ERR_UNKNOWN_ENCODING`. */
function encodingOf(name: unknown): Encoding {
  const encoding = encodingNamed(String(name))
  if (encoding === undefined)
    throw nodeError(
      TypeError,
      'ERR_UNKNOWN_ENCODING',
      `Unknown encoding: ${String(name)}`,
    )
  return encoding
}

/**
 * The encoding `fill` writes a string in: Node's `normalizeEncoding`, which takes null,
 * undefined and '' as utf8, then its refusal of a non-string and of an unknown name.
 */
function fillEncoding(name: unknown): Encoding {
  if (name === undefined || name === null || name === '') return 'utf8'
  if (typeof name !== 'string')
    throw nodeError(
      TypeError,
      'ERR_INVALID_ARG_TYPE',
      `The "encoding" argument must be of type string. ${received(name)}`,
    )
  return encodingOf(name)
}

/** Node's `validateInteger` for one end of a fill: a whole number from 0 to `max`. */
function fillBound(value: unknown, name: string, max: number): number {
  if (typeof value !== 'number')
    throw nodeError(
      TypeError,
      'ERR_INVALID_ARG_TYPE',
      `The "${name}" argument must be of type number. ${received(value)}`,
    )
  if (!Number.isInteger(value)) throw outOfRange(name, 'an integer', value)
  if (value < 0 || value > max)
    throw outOfRange(name, `>= 0 && <= ${max}`, value)
  return value
}

/**
 * Where Node's search for `needle` starts in `length` bytes (node_buffer.cc `IndexOfOffset`): a
 * negative offset counts from the end, an empty needle past the end is found at the end, and -1
 * when nothing can match.
 */
function searchStart(
  length: number,
  offset: number,
  size: number,
  forward: boolean,
): number {
  if (offset < 0) {
    if (offset + length >= 0) return length + offset
    return forward || size === 0 ? 0 : -1
  }
  if (offset + size <= length) return offset
  if (size === 0) return length
  return forward ? -1 : length - 1
}

/**
 * Node 26's search of `haystack` for `needle` (node_buffer.cc `IndexOfString` and
 * `IndexOfBuffer`): the first match at or after `offset` going forward, or the last at or
 * before it going backward, ending by `end`. `length` is the haystack as Node takes it, and
 * `wide` compares utf16le's two-byte units at even positions only, as Node's UCS-2 search
 * does, with a needle cut to whole units.
 */
function search(
  haystack: Uint8Array,
  length: number,
  needle: Uint8Array,
  offset: number,
  end: number,
  forward: boolean,
  wide: boolean,
): number {
  const size = needle.length
  let start = searchStart(length, offset, size, forward)
  let stop = Math.min(Math.max(end, 0), length)
  if (wide) stop -= stop % 2
  if (size === 0) return Math.min(start, stop)
  if (length === 0 || start < 0) return -1
  if (start >= stop) {
    if (forward || stop === 0) return -1
    start = stop - 1
  }
  if ((forward && size + start > stop) || size > stop) return -1
  if (wide && (stop < 2 || size < 2)) return -1
  const step = wide ? 2 : 1
  const span = size - (size % step)
  const last = Math.floor(stop / step) - span / step
  const first = Math.floor(start / step)
  const matches = (unit: number) => {
    for (let index = 0; index < span; index++)
      if (haystack[unit * step + index] !== needle[index]) return false
    return true
  }
  if (forward) {
    for (let unit = first; unit <= last; unit++)
      if (matches(unit)) return unit * step
  } else
    for (let unit = Math.min(first, last); unit >= 0; unit--)
      if (matches(unit)) return unit * step
  return -1
}

/** A string's bytes, with Node's rule for a missing or non-string encoding (utf8). */
function stringBytes(text: string, encoding: unknown): Uint8Array {
  return typeof encoding === 'string' && encoding.length > 0
    ? encode(text, encodingOf(encoding))
    : encode(text, 'utf8')
}

function isArrayBuffer(value: unknown): value is ArrayBufferLike {
  return (
    value instanceof ArrayBuffer ||
    (typeof SharedArrayBuffer === 'function' &&
      value instanceof SharedArrayBuffer)
  )
}

const FROM_TYPES =
  'The first argument must be of type string or an instance of Buffer, ArrayBuffer, or Array or an Array-like Object.'

export class Buffer extends Uint8Array {
  /**
   * `new Buffer(string, encoding)` is the string's bytes, as Node's deprecated constructor
   * gives them, where Uint8Array's would read the string as a length. Every other form is
   * Uint8Array's own, which Node's matches and which species and views build a Buffer with.
   */
  constructor(...args: unknown[]) {
    const [value, encoding] = args
    // Typed as Uint8Array's (buffer, offset, length) form; at run time each of its forms
    // passes through, and a string's bytes are a view Uint8Array copies.
    const form = (
      typeof value === 'string' ? [stringBytes(value, encoding)] : args
    ) as [ArrayBuffer, number?, number?]
    super(...form)
  }

  /**
   * Node's `Buffer.from`, in Node's order: a string's bytes in `encoding` (utf8 by default); a
   * view over an ArrayBuffer (`byteOffset`, `length`) that shares its memory; what an object's
   * `valueOf` gives when that is a string or another object; a copy of an array-like (a typed
   * array included), each element converted as a Uint8Array converts it; `{ type: 'Buffer',
   * data }`; or the string an object's `Symbol.toPrimitive` gives.
   */
  static override from(
    value: unknown,
    encodingOrOffset?: unknown,
    length?: unknown,
  ): Buffer {
    if (typeof value === 'string')
      return Buffer.#copy(stringBytes(value, encodingOrOffset))
    if (typeof value === 'object' && value !== null) {
      if (isArrayBuffer(value))
        return Buffer.#view(value, encodingOrOffset, length)
      const unwrap: unknown = Reflect.get(value, 'valueOf')
      const primitive: unknown =
        typeof unwrap === 'function'
          ? Reflect.apply(unwrap, value, [])
          : undefined
      if (
        primitive !== null &&
        primitive !== undefined &&
        primitive !== value &&
        (typeof primitive === 'string' || typeof primitive === 'object')
      )
        return Buffer.from(primitive, encodingOrOffset, length)
      const count: unknown = Reflect.get(value, 'length')
      if (count !== undefined || isArrayBuffer(Reflect.get(value, 'buffer')))
        return typeof count === 'number'
          ? Buffer.#arrayLike(value, count)
          : new Buffer(0)
      const data: unknown = Reflect.get(value, 'data')
      if (Reflect.get(value, 'type') === 'Buffer' && Array.isArray(data))
        return Buffer.#arrayLike(data, data.length)
      const toPrimitive: unknown = Reflect.get(value, Symbol.toPrimitive)
      if (typeof toPrimitive === 'function') {
        const text: unknown = Reflect.apply(toPrimitive, value, ['string'])
        if (typeof text === 'string')
          return Buffer.#copy(stringBytes(text, encodingOrOffset))
      }
    }
    throw nodeError(
      TypeError,
      'ERR_INVALID_ARG_TYPE',
      `${FROM_TYPES} ${received(value)}`,
    )
  }

  /** Node's `Buffer.alloc`: `size` zero bytes, or `fill` over them as `buffer.fill` writes it. */
  static alloc(size: unknown, fill?: unknown, encoding?: unknown): Buffer {
    if (typeof size !== 'number')
      throw nodeError(
        TypeError,
        'ERR_INVALID_ARG_TYPE',
        `The "size" argument must be of type number. ${received(size)}`,
      )
    if (!(size >= 0 && size <= MAX_LENGTH))
      throw outOfRange('size', `>= 0 && <= ${MAX_LENGTH}`, size)
    const buffer = new Buffer(Math.floor(size))
    if (fill === undefined || fill === 0 || !(size > 0)) return buffer
    return buffer.fill(fill, 0, buffer.length, encoding)
  }

  /** Node's `Buffer.allocUnsafe`; zeroed here, which Node allows. */
  static allocUnsafe(size: unknown): Buffer {
    return Buffer.alloc(size)
  }

  static isBuffer(value: unknown): value is Buffer {
    return value instanceof Buffer
  }

  static isEncoding(encoding: unknown): boolean {
    return typeof encoding === 'string' && encodingNamed(encoding) !== undefined
  }

  /** The bytes `value` takes: a string's in `encoding`, or a buffer's or view's own. */
  static byteLength(value: unknown, encoding?: unknown): number {
    if (typeof value === 'string') return stringBytes(value, encoding).length
    if (ArrayBuffer.isView(value) || isArrayBuffer(value))
      return value.byteLength
    throw nodeError(
      TypeError,
      'ERR_INVALID_ARG_TYPE',
      `The "string" argument must be of type string or an instance of Buffer or ArrayBuffer. ${received(value)}`,
    )
  }

  /** Node's `Buffer.concat`: the list's bytes in order, cut or zero-padded to `totalLength`. */
  static concat(list: unknown, totalLength?: unknown): Buffer {
    if (!Array.isArray(list))
      throw nodeError(
        TypeError,
        'ERR_INVALID_ARG_TYPE',
        `The "list" argument must be an instance of Array. ${received(list)}`,
      )
    const parts: Uint8Array[] = list.map((part, index) => {
      if (part instanceof Uint8Array) return part
      throw nodeError(
        TypeError,
        'ERR_INVALID_ARG_TYPE',
        `The "list[${index}]" argument must be an instance of Buffer or Uint8Array. ${received(part)}`,
      )
    })
    const size =
      totalLength === undefined
        ? parts.reduce((sum, part) => sum + part.length, 0)
        : Number(totalLength)
    const out = Buffer.alloc(size)
    let at = 0
    for (const part of parts) {
      if (at >= out.length) break
      out.set(part.subarray(0, out.length - at), at)
      at += part.length
    }
    return out
  }

  /** The bytes from `start` to `end` in `encoding` (utf8 by default), as Node's toString. */
  override toString(
    encoding?: unknown,
    start?: unknown,
    end?: unknown,
  ): string {
    const length = this.length
    let from = Number(start ?? 0)
    if (!(from > 0)) from = 0
    else if (from >= length) return ''
    else from = Math.trunc(from)
    let to = end === undefined ? length : Number(end)
    to = to > length ? length : Math.trunc(to) || 0
    if (to <= from) return ''
    const bytes = this.subarray(from, to)
    return encoding === undefined
      ? decode(bytes, 'utf8')
      : decode(bytes, encodingOf(encoding))
  }

  /**
   * Node's `fill`, not Uint8Array's, which turns a string into 0: `value` repeated from `offset`
   * to `end`. A string is written in `encoding` (utf8 by default, and `offset` or `end` may name
   * it instead), a view as its bytes and anything else as one byte. Node's checks come in Node's
   * order: the encoding, then the range (a whole number in bounds, or a TypeError or RangeError),
   * then a value that has no bytes, which throws rather than leave the range as it was.
   */
  override fill(
    value: unknown,
    offset?: unknown,
    end?: unknown,
    encoding?: unknown,
  ): this {
    let [from, to, named] = [offset, end, encoding]
    if (typeof value === 'string') {
      if (from === undefined || typeof from === 'string')
        [from, to, named] = [0, this.length, from]
      else if (typeof to === 'string') [to, named] = [this.length, to]
    }
    const text = typeof value === 'string' ? fillEncoding(named) : undefined
    let start = 0
    let stop = this.length
    if (from !== undefined) {
      start = fillBound(from, 'offset', MAX_LENGTH)
      stop = to === undefined ? this.length : fillBound(to, 'end', this.length)
      if (start >= stop) return this
    }
    if (text === undefined || value === '') {
      if (ArrayBuffer.isView(value))
        this.#repeat(
          new Uint8Array(value.buffer, value.byteOffset, value.byteLength),
          value,
          start,
          stop,
        )
      // '' fills zeros, as Node's does; anything else is the byte its number gives.
      else
        Reflect.apply(Uint8Array.prototype.fill, this, [
          value === '' ? 0 : value,
          start,
          stop,
        ])
      return this
    }
    this.#repeat(encode(String(value), text), value, start, stop)
    return this
  }

  /** `pattern` from `start`, then copies of what is written until `end`; `value` names it. */
  #repeat(
    pattern: Uint8Array,
    value: unknown,
    start: number,
    end: number,
  ): void {
    if (pattern.length === 0) throw invalidValue(value)
    let filled = Math.min(pattern.length, end - start)
    this.set(pattern.subarray(0, filled), start)
    while (start + filled < end) {
      const copied = Math.min(filled, end - start - filled)
      this.copyWithin(start + filled, start, start + copied)
      filled += copied
    }
  }

  /**
   * Node 26's `indexOf(value, byteOffset, end, encoding)`, not Uint8Array's, which finds only a
   * number equal to a byte: a string's bytes in `encoding`, a Uint8Array's bytes, or a number's
   * low byte, searched from `byteOffset` up to `end`.
   */
  override indexOf(
    value: unknown,
    byteOffset?: unknown,
    end?: unknown,
    encoding?: unknown,
  ): number {
    return this.#search(value, byteOffset, end, encoding, true)
  }

  /** Node 26's `lastIndexOf`: as `indexOf`, the last match at or before `byteOffset`. */
  override lastIndexOf(
    value: unknown,
    byteOffset?: unknown,
    end?: unknown,
    encoding?: unknown,
  ): number {
    return this.#search(value, byteOffset, end, encoding, false)
  }

  /** Node 26's `includes`: whether `indexOf` finds `value`. */
  override includes(
    value: unknown,
    byteOffset?: unknown,
    end?: unknown,
    encoding?: unknown,
  ): boolean {
    return this.#search(value, byteOffset, end, encoding, true) !== -1
  }

  /**
   * Node 26's `bidirectionalIndexOf`, with its callers' reading of `end`: a string `end` or
   * `byteOffset` names the encoding, the offset is clamped to 32 bits and NaN searches all of
   * the buffer, and an unknown encoding throws for a string only. Node aborts the process on an
   * `end` that is not a number; here it is read as one.
   */
  #search(
    value: unknown,
    byteOffset: unknown,
    end: unknown,
    encoding: unknown,
    forward: boolean,
  ): number {
    let [offset, stop, named] = [byteOffset, end, encoding]
    if (typeof stop === 'string') [stop, named] = [this.length, stop]
    else if (stop === undefined) stop = this.length
    if (typeof offset === 'string') [offset, named] = [undefined, offset]
    let from = Math.min(Math.max(Number(offset), -0x80000000), 0x7fffffff)
    if (Number.isNaN(from)) from = forward ? 0 : this.length
    const until = Math.trunc(Number(stop)) || 0
    const at = Math.trunc(from)
    if (typeof value === 'number') {
      const byte = Uint8Array.of((value >>> 0) & 0xff)
      return search(this, this.length, byte, at, until, forward, false)
    }
    const known = named === undefined ? 'utf8' : encodingNamed(String(named))
    if (typeof value === 'string') {
      const text = known ?? encodingOf(named)
      const wide = text === 'utf16le'
      const length = wide ? this.length - (this.length % 2) : this.length
      const needle = encode(value, text)
      return search(this, length, needle, at, until, forward, wide)
    }
    if (value instanceof Uint8Array)
      return search(
        this,
        this.length,
        value,
        at,
        until,
        forward,
        known === 'utf16le',
      )
    throw nodeError(
      TypeError,
      'ERR_INVALID_ARG_TYPE',
      `The "value" argument must be one of type number or string or an instance of Buffer or Uint8Array. ${received(value)}`,
    )
  }

  /** Node's `toLocaleString`, which is its `toString`, where Uint8Array's lists the numbers. */
  override toLocaleString(
    encoding?: unknown,
    start?: unknown,
    end?: unknown,
  ): string {
    return this.toString(encoding, start, end)
  }

  /** Whether `other` holds the same bytes. */
  equals(other: unknown): boolean {
    if (!(other instanceof Uint8Array))
      throw nodeError(
        TypeError,
        'ERR_INVALID_ARG_TYPE',
        `The "otherBuffer" argument must be an instance of Buffer or Uint8Array. ${received(other)}`,
      )
    if (other.length !== this.length) return false
    return this.every((byte, index) => byte === other[index])
  }

  /** Unsigned image-header fields: PNG uses big endian, RIFF/WebP uses little endian. */
  readUInt8(offset: unknown = 0): number {
    return this.#readUnsigned(offset, 1, 'be')
  }

  readUInt16BE(offset: unknown = 0): number {
    return this.#readUnsigned(offset, 2, 'be')
  }

  readUInt16LE(offset: unknown = 0): number {
    return this.#readUnsigned(offset, 2, 'le')
  }

  readUInt32BE(offset: unknown = 0): number {
    return this.#readUnsigned(offset, 4, 'be')
  }

  readUInt32LE(offset: unknown = 0): number {
    return this.#readUnsigned(offset, 4, 'le')
  }

  /** PNG compares its CRC as a signed 32-bit value. */
  readInt32BE(offset: unknown = 0): number {
    return this.readUInt32BE(offset) | 0
  }

  #readUnsigned(offset: unknown, width: number, order: 'be' | 'le'): number {
    if (typeof offset !== 'number')
      throw nodeError(
        TypeError,
        'ERR_INVALID_ARG_TYPE',
        `The "offset" argument must be of type number. ${received(offset)}`,
      )
    if (Math.floor(offset) !== offset)
      throw outOfRange('offset', 'an integer', offset)
    const last = this.length - width
    if (last < 0)
      throw nodeError(
        RangeError,
        'ERR_BUFFER_OUT_OF_BOUNDS',
        'Attempt to access memory outside buffer bounds',
      )
    if (offset < 0 || offset > last)
      throw outOfRange('offset', `>= 0 and <= ${last}`, offset)
    // Index this view, not its ArrayBuffer: a sliced screenshot may start after byte zero.
    // Multiplication keeps high-bit 32-bit fields unsigned instead of JS bitwise signed ints.
    let value = 0
    for (let index = 0; index < width; index++) {
      const at = order === 'be' ? offset + index : offset + width - index - 1
      value = value * 256 + (this[at] ?? 0)
    }
    return value
  }

  /** Node's `slice`, which shares memory as `subarray` does, unlike Uint8Array's copy. */
  override slice(start?: number, end?: number): Buffer {
    return this.subarray(start, end)
  }

  /** A view as a Buffer, as Node's; typed for callers, since species already makes one. */
  override subarray(start?: number, end?: number): Buffer {
    const view = super.subarray(start, end)
    return new Buffer(view.buffer, view.byteOffset, view.length)
  }

  static #copy(bytes: Uint8Array): Buffer {
    const buffer = new Buffer(bytes.length)
    buffer.set(bytes)
    return buffer
  }

  /** Node's `fromArrayBuffer`: a view at `offset` (NaN as 0), as long as fits by default. */
  static #view(
    memory: ArrayBufferLike,
    offset: unknown,
    length: unknown,
  ): Buffer {
    const start = offset === undefined ? 0 : Number(offset) || 0
    const room = memory.byteLength - start
    if (room < 0) throw Buffer.#outOfBounds('offset')
    let size = room
    if (length !== undefined) {
      size = Number(length) > 0 ? Number(length) : 0
      if (size > room) throw Buffer.#outOfBounds('length')
    }
    // The typed constructor takes an ArrayBuffer only; a view over a SharedArrayBuffer is as
    // valid, so it is built untyped.
    const view: Buffer = Reflect.construct(Buffer, [memory, start, size])
    return view
  }

  static #outOfBounds(name: string): Error {
    return nodeError(
      RangeError,
      'ERR_BUFFER_OUT_OF_BOUNDS',
      `"${name}" is outside of buffer bounds`,
    )
  }

  /** Each item converted as a Uint8Array converts it (ToNumber, then modulo 256). */
  static #arrayLike(items: object, count: number): Buffer {
    const size = Math.max(0, Math.trunc(count) || 0)
    const buffer = new Buffer(size)
    for (let index = 0; index < size; index++)
      buffer[index] = Number(Reflect.get(items, index))
    return buffer
  }
}
